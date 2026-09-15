//! Native Vortex tables, compressed file encoding, and projected scans.
//!
//! This module runs outside the render loop. It deliberately returns independent
//! node/edge files so either relation can be scanned without reading the other.
//! A durable store must publish both files as one generation (for example inside
//! a DuckDB transaction), rather than independently overwriting live files.

use std::fmt;
use std::path::Path;
use vortex::array::arrays::struct_::StructArrayExt;
use vortex::array::arrays::{BoolArray, PrimitiveArray, StructArray, VarBinViewArray};
use vortex::array::expr::Expression;
use vortex::array::stream::ArrayStreamExt;
use vortex::array::{ArrayRef, IntoArray, VortexSessionExecute};
use vortex::buffer::{ByteBuffer, ByteBufferMut};
use vortex::dtype::{DType, Nullability, PType, StructFields};
use vortex::error::VortexError;
use vortex::file::{OpenOptionsSessionExt, VortexFile, WriteOptionsSessionExt};
use vortex::session::VortexSession;
use vortex::VortexSessionDefault;

use crate::{Edge, EdgeId, GraphError, GraphSnapshot, Node, NodeId, WorldRect};

/// Failure before a new graph snapshot can be published.
#[derive(Debug)]
pub enum VortexGraphError {
    Vortex(Box<VortexError>),
    Graph(GraphError),
    Schema {
        table: &'static str,
        expected: DType,
        actual: DType,
    },
    RowLimit {
        table: &'static str,
        rows: u64,
        limit: usize,
    },
}

impl fmt::Display for VortexGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vortex(error) => write!(f, "vortex IO: {error}"),
            Self::Graph(error) => write!(f, "invalid stored graph: {error}"),
            Self::Schema {
                table,
                expected,
                actual,
            } => write!(
                f,
                "invalid {table} schema: expected {expected}, got {actual}"
            ),
            Self::RowLimit { table, rows, limit } => {
                write!(f, "{table} has {rows} rows, exceeding limit {limit}")
            }
        }
    }
}
impl std::error::Error for VortexGraphError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Vortex(e) => Some(e.as_ref()),
            Self::Graph(e) => Some(e),
            _ => None,
        }
    }
}
impl From<VortexError> for VortexGraphError {
    fn from(error: VortexError) -> Self {
        Self::Vortex(Box::new(error))
    }
}
impl From<GraphError> for VortexGraphError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

/// Two standalone, compressed `.vortex` files with the v1 graph table schemas.
/// Buffer clones share storage. File bytes are compatible with Vortex tools.
#[derive(Clone, Debug)]
pub struct VortexGraphFiles {
    pub nodes: ByteBuffer,
    pub edges: ByteBuffer,
}

/// Reject oversized tables before whole-graph materialization.
/// These limits bound row counts, not the bytes of variable-length strings.
#[derive(Clone, Copy, Debug)]
pub struct ReadLimits {
    pub nodes: usize,
    pub edges: usize,
}
impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            nodes: 1_000_000,
            edges: 10_000_000,
        }
    }
}

/// V1 node schema: non-null struct, u64 ID, f64 world geometry, UTF-8 labels
/// and renderer keys, boolean visibility. Field order is part of the schema.
pub fn node_dtype() -> DType {
    use Nullability::NonNullable as N;
    DType::Struct(
        StructFields::from_iter([
            ("id", DType::Primitive(PType::U64, N)),
            ("x", DType::Primitive(PType::F64, N)),
            ("y", DType::Primitive(PType::F64, N)),
            ("width", DType::Primitive(PType::F64, N)),
            ("height", DType::Primitive(PType::F64, N)),
            ("label", DType::Utf8(N)),
            ("renderer", DType::Utf8(N)),
            ("hidden", DType::Bool(N)),
        ]),
        N,
    )
}

/// V1 edge schema uses stable endpoint IDs, never snapshot-local indices.
/// Nullable handle strings distinguish an absent handle from an empty name.
pub fn edge_dtype() -> DType {
    use Nullability::{NonNullable as N, Nullable as Y};
    DType::Struct(
        StructFields::from_iter([
            ("id", DType::Primitive(PType::U64, N)),
            ("source", DType::Primitive(PType::U64, N)),
            ("target", DType::Primitive(PType::U64, N)),
            ("source_handle", DType::Utf8(Y)),
            ("target_handle", DType::Utf8(Y)),
            ("weight", DType::Primitive(PType::F64, N)),
        ]),
        N,
    )
}

/// Convert a validated snapshot to native Vortex struct arrays. Numeric data is
/// columnar; strings remain separate from geometry and topology. O(V + E).
pub fn graph_arrays(graph: &GraphSnapshot) -> Result<(ArrayRef, ArrayRef), VortexGraphError> {
    let nodes = graph.nodes();
    let edges = graph.edges();
    let nodes = StructArray::from_fields(&[
        (
            "id",
            nodes
                .iter()
                .map(|n| n.id.0)
                .collect::<PrimitiveArray>()
                .into_array(),
        ),
        (
            "x",
            nodes
                .iter()
                .map(|n| n.bounds.x)
                .collect::<PrimitiveArray>()
                .into_array(),
        ),
        (
            "y",
            nodes
                .iter()
                .map(|n| n.bounds.y)
                .collect::<PrimitiveArray>()
                .into_array(),
        ),
        (
            "width",
            nodes
                .iter()
                .map(|n| n.bounds.width)
                .collect::<PrimitiveArray>()
                .into_array(),
        ),
        (
            "height",
            nodes
                .iter()
                .map(|n| n.bounds.height)
                .collect::<PrimitiveArray>()
                .into_array(),
        ),
        (
            "label",
            VarBinViewArray::from_iter_str(nodes.iter().map(|n| &n.label)).into_array(),
        ),
        (
            "renderer",
            VarBinViewArray::from_iter_str(nodes.iter().map(|n| &n.renderer)).into_array(),
        ),
        (
            "hidden",
            nodes
                .iter()
                .map(|n| n.hidden)
                .collect::<BoolArray>()
                .into_array(),
        ),
    ])?
    .into_array();
    let edges = StructArray::from_fields(&[
        (
            "id",
            edges
                .iter()
                .map(|e| e.id.0)
                .collect::<PrimitiveArray>()
                .into_array(),
        ),
        (
            "source",
            edges
                .iter()
                .map(|e| e.source.0)
                .collect::<PrimitiveArray>()
                .into_array(),
        ),
        (
            "target",
            edges
                .iter()
                .map(|e| e.target.0)
                .collect::<PrimitiveArray>()
                .into_array(),
        ),
        (
            "source_handle",
            VarBinViewArray::from_iter_nullable_str(
                edges.iter().map(|e| e.source_handle.as_deref()),
            )
            .into_array(),
        ),
        (
            "target_handle",
            VarBinViewArray::from_iter_nullable_str(
                edges.iter().map(|e| e.target_handle.as_deref()),
            )
            .into_array(),
        ),
        (
            "weight",
            edges
                .iter()
                .map(|e| e.weight)
                .collect::<PrimitiveArray>()
                .into_array(),
        ),
    ])?
    .into_array();
    Ok((nodes, edges))
}

/// A reusable Vortex session. Keep this on a worker; no GPUI dependency or runtime
/// is created here. Async file operations run in the caller's runtime.
pub struct VortexGraphIo {
    session: VortexSession,
}

impl Default for VortexGraphIo {
    fn default() -> Self {
        Self {
            session: VortexSession::default(),
        }
    }
}

impl VortexGraphIo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Encode both relations using Vortex's default adaptive compression/layout.
    pub async fn encode(
        &self,
        graph: &GraphSnapshot,
    ) -> Result<VortexGraphFiles, VortexGraphError> {
        let (nodes, edges) = graph_arrays(graph)?;
        Ok(VortexGraphFiles {
            nodes: self.encode_table(nodes).await?,
            edges: self.encode_table(edges).await?,
        })
    }

    /// Encode an arbitrary Vortex array as a standalone file.
    pub async fn encode_table(&self, array: ArrayRef) -> Result<ByteBuffer, VortexGraphError> {
        let mut bytes = ByteBufferMut::empty();
        self.session
            .write_options()
            .write(&mut bytes, array.to_array_stream())
            .await?;
        Ok(bytes.freeze())
    }

    /// Push a filter and projection into the Vortex scanner, returning native
    /// arrays. Expressions are checked against the file dtype before execution.
    ///
    /// This materializes the result; callers should supply selective predicates
    /// and projections for large files. Use `decode` for bounded full-graph reads.
    pub async fn scan(
        &self,
        bytes: ByteBuffer,
        filter: Option<Expression>,
        projection: Option<Expression>,
    ) -> Result<ArrayRef, VortexGraphError> {
        let file = self.session.open_options().open_buffer(bytes)?;
        Self::scan_open(file, filter, projection).await
    }

    /// Scan a file using Vortex's range-reading IO, without first loading the
    /// entire compressed file into memory. Result arrays are materialized.
    pub async fn scan_path(
        &self,
        path: impl AsRef<Path>,
        filter: Option<Expression>,
        projection: Option<Expression>,
    ) -> Result<ArrayRef, VortexGraphError> {
        let file = self.session.open_options().open_path(path).await?;
        Self::scan_open(file, filter, projection).await
    }

    async fn scan_open(
        file: VortexFile,
        filter: Option<Expression>,
        projection: Option<Expression>,
    ) -> Result<ArrayRef, VortexGraphError> {
        let mut scan = file.scan()?;
        if let Some(filter) = filter {
            scan = scan.with_filter(
                filter
                    .optimize_recursive(file.dtype())?
                    .bind(file.dtype())?,
            );
        }
        if let Some(projection) = projection {
            scan = scan.with_projection(
                projection
                    .optimize_recursive(file.dtype())?
                    .bind(file.dtype())?,
            );
        }
        Ok(scan.into_array_stream()?.read_all().await?)
    }

    /// Validate schemas and declared row limits before materializing either table,
    /// then validate the full graph (including cross-table endpoint references).
    pub async fn decode(
        &self,
        files: VortexGraphFiles,
        limits: ReadLimits,
    ) -> Result<GraphSnapshot, VortexGraphError> {
        let nodes = self.session.open_options().open_buffer(files.nodes)?;
        let edges = self.session.open_options().open_buffer(files.edges)?;
        self.decode_open(nodes, edges, limits).await
    }

    /// Open a pair of immutable node/edge files from the same saved generation.
    /// Callers must not overwrite either file while a read is in progress.
    pub async fn read_paths(
        &self,
        nodes: impl AsRef<Path>,
        edges: impl AsRef<Path>,
        limits: ReadLimits,
    ) -> Result<GraphSnapshot, VortexGraphError> {
        let nodes = self.session.open_options().open_path(nodes).await?;
        let edges = self.session.open_options().open_path(edges).await?;
        self.decode_open(nodes, edges, limits).await
    }

    async fn decode_open(
        &self,
        nodes: VortexFile,
        edges: VortexFile,
        limits: ReadLimits,
    ) -> Result<GraphSnapshot, VortexGraphError> {
        for (table, file, expected, limit) in [
            ("nodes", &nodes, node_dtype(), limits.nodes),
            ("edges", &edges, edge_dtype(), limits.edges),
        ] {
            check_schema(table, file.dtype(), expected)?;
            if file.row_count() > limit as u64 {
                return Err(VortexGraphError::RowLimit {
                    table,
                    rows: file.row_count(),
                    limit,
                });
            }
        }
        let nodes = Self::scan_open(nodes, None, None).await?;
        let edges = Self::scan_open(edges, None, None).await?;
        self.from_arrays(nodes, edges)
    }

    /// Materialize canonical columns and validate before publishing a snapshot.
    /// Compressed arrays are accepted; nullable required fields are rejected.
    pub fn from_arrays(
        &self,
        nodes: ArrayRef,
        edges: ArrayRef,
    ) -> Result<GraphSnapshot, VortexGraphError> {
        check_schema("nodes", nodes.dtype(), node_dtype())?;
        check_schema("edges", edges.dtype(), edge_dtype())?;
        let mut ctx = self.session.create_execution_ctx();
        let nodes = nodes.execute::<StructArray>(&mut ctx)?;
        let edges = edges.execute::<StructArray>(&mut ctx)?;
        // Schema checks above guarantee the physical buffer types and non-nullability.
        let primitive = |table: &StructArray, name: &str, ctx: &mut _| {
            table
                .unmasked_field_by_name(name)?
                .clone()
                .execute::<PrimitiveArray>(ctx)
        };
        let id = primitive(&nodes, "id", &mut ctx)?.to_buffer::<u64>();
        let x = primitive(&nodes, "x", &mut ctx)?.to_buffer::<f64>();
        let y = primitive(&nodes, "y", &mut ctx)?.to_buffer::<f64>();
        let width = primitive(&nodes, "width", &mut ctx)?.to_buffer::<f64>();
        let height = primitive(&nodes, "height", &mut ctx)?.to_buffer::<f64>();
        let labels = nodes
            .unmasked_field_by_name("label")?
            .clone()
            .execute::<VarBinViewArray>(&mut ctx)?;
        let renderers = nodes
            .unmasked_field_by_name("renderer")?
            .clone()
            .execute::<VarBinViewArray>(&mut ctx)?;
        let hidden = nodes
            .unmasked_field_by_name("hidden")?
            .clone()
            .execute::<BoolArray>(&mut ctx)?;
        let mut node_rows = Vec::with_capacity(nodes.len());
        for i in 0..nodes.len() {
            node_rows.push(Node {
                id: NodeId(id[i]),
                bounds: WorldRect {
                    x: x[i],
                    y: y[i],
                    width: width[i],
                    height: height[i],
                },
                label: String::try_from(labels.execute_scalar(i, &mut ctx)?)?,
                renderer: String::try_from(renderers.execute_scalar(i, &mut ctx)?)?,
                hidden: bool::try_from(hidden.execute_scalar(i, &mut ctx)?)?,
            });
        }
        let id = primitive(&edges, "id", &mut ctx)?.to_buffer::<u64>();
        let source = primitive(&edges, "source", &mut ctx)?.to_buffer::<u64>();
        let target = primitive(&edges, "target", &mut ctx)?.to_buffer::<u64>();
        let weight = primitive(&edges, "weight", &mut ctx)?.to_buffer::<f64>();
        let source_handles = edges
            .unmasked_field_by_name("source_handle")?
            .clone()
            .execute::<VarBinViewArray>(&mut ctx)?;
        let target_handles = edges
            .unmasked_field_by_name("target_handle")?
            .clone()
            .execute::<VarBinViewArray>(&mut ctx)?;
        let mut edge_rows = Vec::with_capacity(edges.len());
        for i in 0..edges.len() {
            let source_handle = source_handles.execute_scalar(i, &mut ctx)?;
            let target_handle = target_handles.execute_scalar(i, &mut ctx)?;
            edge_rows.push(Edge {
                id: EdgeId(id[i]),
                source: NodeId(source[i]),
                target: NodeId(target[i]),
                weight: weight[i],
                source_handle: if source_handle.is_null() {
                    None
                } else {
                    Some(String::try_from(source_handle)?)
                },
                target_handle: if target_handle.is_null() {
                    None
                } else {
                    Some(String::try_from(target_handle)?)
                },
            });
        }
        Ok(GraphSnapshot::new(node_rows, edge_rows)?)
    }
}

fn check_schema(
    table: &'static str,
    actual: &DType,
    expected: DType,
) -> Result<(), VortexGraphError> {
    if actual != &expected {
        return Err(VortexGraphError::Schema {
            table,
            expected,
            actual: actual.clone(),
        });
    }
    Ok(())
}
