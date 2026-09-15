//! GPUG query language: typed predicates, endpoint joins, and bounded traversal.
//! See `QUERY_LANGUAGE.md` for grammar, ordering, null, and budget semantics.
#![doc = include_str!("../../QUERY_LANGUAGE.md")]
#[cfg(feature = "duckdb")]
mod duckdb;
mod parser;
#[cfg(feature = "vortex")]
mod vortex;

use crate::{
    Direction, EdgeId, GraphError, GraphSnapshot, Node, NodeId, SearchBudget, SearchWorkspace,
};
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// Candidate relation for a query. Neighborhoods include the seed at depth zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuerySource {
    Nodes,
    Edges,
    Neighbors {
        seed: NodeId,
        direction: Direction,
        hops: usize,
    },
}

/// A compiled query. Parsing validates fields and literal types once, then stores
/// a flat predicate program. Cloning never clones any graph data.
#[derive(Clone, Debug)]
pub struct Query {
    source: QuerySource,
    program: Vec<Instruction>,
    limit: Option<usize>,
}

impl Query {
    pub fn parse(text: &str) -> Result<Self, QueryError> {
        parser::parse(text)
    }
    pub fn source(&self) -> QuerySource {
        self.source
    }
    pub fn limit(&self) -> Option<usize> {
        self.limit
    }
}
impl FromStr for Query {
    type Err = QueryError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

/// Query syntax/type error; `offset` is a UTF-8 byte offset in the source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError {
    pub offset: usize,
    pub message: String,
}
impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "query at byte {}: {}", self.offset, self.message)
    }
}
impl std::error::Error for QueryError {}

#[derive(Clone, Copy, Debug)]
enum NodeField {
    Id,
    Label,
    Renderer,
    Hidden,
    X,
    Y,
    Width,
    Height,
    InDegree,
    OutDegree,
}
#[derive(Clone, Copy, Debug)]
enum Field {
    Node(NodeField),
    Id,
    Source,
    Target,
    SourceNode(NodeField),
    TargetNode(NodeField),
    SourceHandle,
    TargetHandle,
    Weight,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    U64,
    Number,
    Text,
    Bool,
}
impl NodeField {
    fn kind(self) -> Kind {
        match self {
            Self::Id | Self::InDegree | Self::OutDegree => Kind::U64,
            Self::Label | Self::Renderer => Kind::Text,
            Self::Hidden => Kind::Bool,
            _ => Kind::Number,
        }
    }
}
impl Field {
    fn kind(self) -> Kind {
        match self {
            Self::Node(n) | Self::SourceNode(n) | Self::TargetNode(n) => n.kind(),
            Self::Id | Self::Source | Self::Target => Kind::U64,
            Self::SourceHandle | Self::TargetHandle => Kind::Text,
            Self::Weight => Kind::Number,
        }
    }
}
#[derive(Clone, Debug)]
enum Literal {
    U64(u64),
    Number(f64),
    Text(String),
    Bool(bool),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
}
#[derive(Clone, Debug)]
enum Instruction {
    Compare(Field, Op, Literal),
    IsNull(Field),
    Not,
    And,
    Or,
}

enum Value<'a> {
    U64(u64),
    Number(f64),
    Text(&'a str),
    Bool(bool),
    Null,
}

/// Host-controlled work limits. Query text cannot raise these limits.
/// Row evaluation is bounded by `max_rows` and parser-bounded predicate size.
/// Initial workspace sizing and cold graph/index construction are separate costs.
#[derive(Clone, Copy, Debug)]
pub struct QueryBudget {
    pub max_rows: usize,
    pub max_results: usize,
    pub max_traversal_nodes: usize,
    pub max_traversal_edges: usize,
}
impl Default for QueryBudget {
    fn default() -> Self {
        Self {
            max_rows: 100_000,
            max_results: 10_000,
            max_traversal_nodes: 10_000,
            max_traversal_edges: 100_000,
        }
    }
}

/// Borrowed result buffers. IDs refer to the queried snapshot's original entities.
/// Depths are populated only for neighborhood results and align with node IDs.
#[derive(Debug)]
pub struct QueryResult<'a> {
    pub nodes: &'a [NodeId],
    pub edges: &'a [EdgeId],
    pub depths: &'a [usize],
    pub rows_examined: usize,
    pub adjacency_examined: usize,
    /// A host work budget or cancellation stopped execution. False means the query, including
    /// its explicit LIMIT, completed. Never infer absence from a truncated result.
    pub truncated: bool,
}

/// Reusable predicate, traversal and output buffers; owned by one worker at a time.
#[derive(Debug, Default)]
pub struct QueryWorkspace {
    search: SearchWorkspace,
    stack: Vec<Option<bool>>,
    nodes: Vec<NodeId>,
    edges: Vec<EdgeId>,
    depths: Vec<usize>,
}

impl QueryWorkspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluate a compiled query. Unknown neighborhood seeds fail. `WHERE` filters
    /// traversal results, not the traversal frontier. This method does no IO.
    pub fn execute<'a>(
        &'a mut self,
        graph: &GraphSnapshot,
        query: &Query,
        budget: QueryBudget,
    ) -> Result<QueryResult<'a>, GraphError> {
        self.execute_cancellable(graph, query, budget, || false)
    }

    /// Checks cancellation before traversal and every 256 evaluated candidates
    /// or BFS adjacency entries. Cancelled results are marked truncated. The
    /// callback must be cheap and nonblocking; individual string comparisons and
    /// workspace growth remain indivisible operations.
    pub fn execute_cancellable<'a>(
        &'a mut self,
        graph: &GraphSnapshot,
        query: &Query,
        budget: QueryBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<QueryResult<'a>, GraphError> {
        // Validate the seed even for LIMIT 0, and before mutating any result buffer.
        if let QuerySource::Neighbors { seed, .. } = query.source {
            graph
                .node_index(seed)
                .ok_or(GraphError::UnknownNode(seed))?;
        }
        self.nodes.clear();
        self.edges.clear();
        self.depths.clear();
        let mut rows_examined = 0;
        let mut adjacency_examined = 0;
        let mut truncated = false;
        let limit = query.limit.unwrap_or(usize::MAX);
        if limit > 0 {
            let neighborhood = if let QuerySource::Neighbors {
                seed,
                direction,
                hops,
            } = query.source
            {
                let result = self.search.neighborhood_cancellable(
                    graph,
                    seed,
                    direction,
                    SearchBudget {
                        max_hops: hops,
                        max_nodes: budget.max_traversal_nodes,
                        max_edges: budget.max_traversal_edges,
                    },
                    &mut cancelled,
                )?;
                adjacency_examined = result.examined_edges;
                truncated = result.truncated;
                Some(result)
            } else {
                None
            };
            let count = match query.source {
                QuerySource::Nodes => graph.nodes().len(),
                QuerySource::Edges => graph.edges().len(),
                QuerySource::Neighbors { .. } => neighborhood.as_ref().map_or(0, |r| r.nodes.len()),
            };
            for candidate in 0..count {
                let found = self.nodes.len() + self.edges.len();
                if found == limit {
                    break;
                }
                if candidate % 256 == 0 && cancelled() {
                    truncated = true;
                    break;
                }
                if rows_examined == budget.max_rows {
                    truncated = true;
                    break;
                }
                let index = if let Some(result) = &neighborhood {
                    result.indices[candidate] as usize
                } else {
                    candidate
                };
                rows_examined += 1;
                if !matches_row(graph, index, &query.program, &mut self.stack) {
                    continue;
                }
                if found == budget.max_results {
                    truncated = true;
                    break;
                }
                match query.source {
                    QuerySource::Edges => self.edges.push(graph.edges()[index].id),
                    _ => self.nodes.push(graph.nodes()[index].id),
                }
                if let Some(result) = &neighborhood {
                    self.depths.push(result.depths[candidate]);
                }
            }
        }
        Ok(QueryResult {
            nodes: &self.nodes,
            edges: &self.edges,
            depths: &self.depths,
            rows_examined,
            adjacency_examined,
            truncated,
        })
    }
}

fn node_value<'a>(graph: &'a GraphSnapshot, index: usize, field: NodeField) -> Value<'a> {
    let Node {
        id,
        label,
        renderer,
        hidden,
        bounds,
    } = &graph.nodes()[index];
    match field {
        NodeField::Id => Value::U64(id.0),
        NodeField::Label => Value::Text(label),
        NodeField::Renderer => Value::Text(renderer),
        NodeField::Hidden => Value::Bool(*hidden),
        NodeField::X => Value::Number(bounds.x),
        NodeField::Y => Value::Number(bounds.y),
        NodeField::Width => Value::Number(bounds.width),
        NodeField::Height => Value::Number(bounds.height),
        NodeField::InDegree => Value::U64(graph.incoming.incident(index).len() as u64),
        NodeField::OutDegree => Value::U64(graph.outgoing.incident(index).len() as u64),
    }
}
fn value(graph: &GraphSnapshot, index: usize, field: Field) -> Value<'_> {
    if let Field::Node(field) = field {
        return node_value(graph, index, field);
    }
    let edge = &graph.edges()[index];
    match field {
        Field::Id => Value::U64(edge.id.0),
        Field::Source => Value::U64(edge.source.0),
        Field::Target => Value::U64(edge.target.0),
        Field::SourceHandle => edge
            .source_handle
            .as_deref()
            .map_or(Value::Null, Value::Text),
        Field::TargetHandle => edge
            .target_handle
            .as_deref()
            .map_or(Value::Null, Value::Text),
        Field::Weight => Value::Number(edge.weight),
        Field::SourceNode(field) => node_value(graph, graph.sources[index] as usize, field),
        Field::TargetNode(field) => node_value(graph, graph.targets[index] as usize, field),
        Field::Node(_) => unreachable!("node field handled above"),
    }
}
fn compare(value: Value<'_>, op: Op, literal: &Literal) -> Option<bool> {
    if matches!(value, Value::Null) {
        return None;
    }
    let order = match (value, literal) {
        (Value::U64(a), Literal::U64(b)) => a.cmp(b),
        (Value::Number(a), Literal::Number(b)) => {
            a.partial_cmp(b).expect("finite graph and query numbers")
        }
        (Value::Bool(a), Literal::Bool(b)) => a.cmp(b),
        (Value::Text(a), Literal::Text(b)) => {
            if op == Op::Contains {
                return Some(a.contains(b.as_str()));
            }
            a.cmp(b)
        }
        _ => unreachable!("parser checks field and literal types"),
    };
    Some(match op {
        Op::Eq => order == Ordering::Equal,
        Op::Ne => order != Ordering::Equal,
        Op::Lt => order == Ordering::Less,
        Op::Le => order != Ordering::Greater,
        Op::Gt => order == Ordering::Greater,
        Op::Ge => order != Ordering::Less,
        Op::Contains => unreachable!("CONTAINS requires text"),
    })
}
fn matches_row(
    graph: &GraphSnapshot,
    index: usize,
    program: &[Instruction],
    stack: &mut Vec<Option<bool>>,
) -> bool {
    if program.is_empty() {
        return true;
    }
    stack.clear();
    for instruction in program {
        match instruction {
            Instruction::Compare(field, op, literal) => {
                stack.push(compare(value(graph, index, *field), *op, literal))
            }
            Instruction::IsNull(field) => {
                stack.push(Some(matches!(value(graph, index, *field), Value::Null)))
            }
            Instruction::Not => {
                let a = stack.pop().expect("validated predicate stack");
                stack.push(a.map(|a| !a));
            }
            Instruction::And | Instruction::Or => {
                let b = stack.pop().expect("validated predicate stack");
                let a = stack.pop().expect("validated predicate stack");
                stack.push(if matches!(instruction, Instruction::And) {
                    match (a, b) {
                        (Some(false), _) | (_, Some(false)) => Some(false),
                        (Some(true), Some(true)) => Some(true),
                        _ => None,
                    }
                } else {
                    match (a, b) {
                        (Some(true), _) | (_, Some(true)) => Some(true),
                        (Some(false), Some(false)) => Some(false),
                        _ => None,
                    }
                });
            }
        }
    }
    stack.last() == Some(&Some(true))
}
