use super::*;
use ::duckdb::types::Value;

impl Query {
    /// Internal typed translation. Every identifier/operator comes from enums;
    /// user literal values remain bound parameters, never SQL source text.
    pub(crate) fn duckdb_select(&self) -> Option<(String, Vec<Value>)> {
        if matches!(self.source, QuerySource::Neighbors { .. }) {
            return None;
        }
        let mut stack: Vec<String> = Vec::new();
        let mut values = Vec::new();
        let mut join_source = false;
        let mut join_target = false;
        for instruction in &self.program {
            match instruction {
                Instruction::Compare(field, op, literal) => {
                    join_source |= matches!(field, Field::SourceNode(_));
                    join_target |= matches!(field, Field::TargetNode(_));
                    let field = column(*field);
                    values.push(match literal {
                        Literal::U64(v) => (*v).into(),
                        Literal::Number(v) => (*v).into(),
                        Literal::Text(v) => v.clone().into(),
                        Literal::Bool(v) => (*v).into(),
                    });
                    stack.push(if *op == Op::Contains {
                        format!("contains({field}, ?)")
                    } else {
                        let op = match op {
                            Op::Eq => "=",
                            Op::Ne => "<>",
                            Op::Lt => "<",
                            Op::Le => "<=",
                            Op::Gt => ">",
                            Op::Ge => ">=",
                            Op::Contains => unreachable!(),
                        };
                        format!("({field} {op} ?)")
                    });
                }
                Instruction::IsNull(field) => {
                    join_source |= matches!(field, Field::SourceNode(_));
                    join_target |= matches!(field, Field::TargetNode(_));
                    stack.push(format!("({} IS NULL)", column(*field)));
                }
                Instruction::Not => {
                    let a = stack.pop()?;
                    stack.push(format!("(NOT {a})"));
                }
                Instruction::And | Instruction::Or => {
                    let b = stack.pop()?;
                    let a = stack.pop()?;
                    let op = if matches!(instruction, Instruction::And) {
                        "AND"
                    } else {
                        "OR"
                    };
                    stack.push(format!("({a} {op} {b})"));
                }
            }
        }
        let filter = stack.pop().unwrap_or_else(|| "TRUE".into());
        let mut sql = if self.source == QuerySource::Edges {
            "SELECT e.id FROM gpug_data.edges e".to_owned()
        } else {
            "SELECT n.id FROM gpug_data.nodes n".to_owned()
        };
        if join_source {
            sql.push_str(" JOIN gpug_data.nodes s ON s.id = e.source");
        }
        if join_target {
            sql.push_str(" JOIN gpug_data.nodes t ON t.id = e.target");
        }
        sql.push_str(" WHERE ");
        sql.push_str(&filter);
        sql.push_str(if self.source == QuerySource::Edges {
            " ORDER BY e.ordinal"
        } else {
            " ORDER BY n.ordinal"
        });
        Some((sql, values))
    }
}
fn node_column(field: NodeField, alias: &str) -> String {
    let name = match field {
        NodeField::Id => "id",
        NodeField::Label => "label",
        NodeField::Renderer => "renderer",
        NodeField::Hidden => "hidden",
        NodeField::X => "x",
        NodeField::Y => "y",
        NodeField::Width => "width",
        NodeField::Height => "height",
        NodeField::InDegree | NodeField::OutDegree => {
            let endpoint = if matches!(field, NodeField::InDegree) {
                "target"
            } else {
                "source"
            };
            return format!(
                "(SELECT count(*) FROM gpug_data.edges d WHERE d.{endpoint} = {alias}.id)"
            );
        }
    };
    format!("{alias}.{name}")
}
fn column(field: Field) -> String {
    match field {
        Field::Node(n) => node_column(n, "n"),
        Field::SourceNode(n) => node_column(n, "s"),
        Field::TargetNode(n) => node_column(n, "t"),
        Field::Id => "e.id".into(),
        Field::Source => "e.source".into(),
        Field::Target => "e.target".into(),
        Field::Weight => "e.weight".into(),
        Field::SourceHandle => "e.source_handle".into(),
        Field::TargetHandle => "e.target_handle".into(),
    }
}
