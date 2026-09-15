use super::*;
use ::vortex::array::expr::{self, Expression};

impl Query {
    /// Translate a complete predicate for a single Vortex node/edge table.
    /// Returns `None` for traversal, degree/endpoint joins, and CONTAINS; the host
    /// must use in-memory evaluation in those cases. No clauses are discarded.
    ///
    /// The expression only implements WHERE, not result LIMIT or host budgets.
    /// For tables using the v1 schema, it has the same null and comparison semantics
    /// as `QueryWorkspace`. Projection/order/limits remain the caller's responsibility.
    pub fn vortex_filter(&self) -> Option<Expression> {
        if matches!(self.source, QuerySource::Neighbors { .. }) {
            return None;
        }
        let mut stack = Vec::new();
        for instruction in &self.program {
            match instruction {
                Instruction::Compare(field, op, literal) => {
                    let field = column(*field)?;
                    let literal = match literal {
                        Literal::U64(v) => expr::lit(*v),
                        Literal::Number(v) => expr::lit(*v),
                        Literal::Text(v) => expr::lit(v.as_str()),
                        Literal::Bool(v) => expr::lit(*v),
                    };
                    let result = match op {
                        Op::Eq => expr::eq(field, literal),
                        Op::Ne => expr::not_eq(field, literal),
                        Op::Lt => expr::lt(field, literal),
                        Op::Le => expr::lt_eq(field, literal),
                        Op::Gt => expr::gt(field, literal),
                        Op::Ge => expr::gt_eq(field, literal),
                        Op::Contains => return None,
                    };
                    stack.push(result);
                }
                Instruction::IsNull(field) => stack.push(expr::is_null(column(*field)?)),
                Instruction::Not => {
                    let a = stack.pop()?;
                    stack.push(expr::not(a));
                }
                Instruction::And | Instruction::Or => {
                    let b = stack.pop()?;
                    let a = stack.pop()?;
                    stack.push(if matches!(instruction, Instruction::And) {
                        expr::and(a, b)
                    } else {
                        expr::or(a, b)
                    });
                }
            }
        }
        Some(stack.pop().unwrap_or_else(|| expr::lit(true)))
    }
}

fn column(field: Field) -> Option<Expression> {
    let name = match field {
        Field::Node(field) => match field {
            NodeField::Id => "id",
            NodeField::Label => "label",
            NodeField::Renderer => "renderer",
            NodeField::Hidden => "hidden",
            NodeField::X => "x",
            NodeField::Y => "y",
            NodeField::Width => "width",
            NodeField::Height => "height",
            _ => return None,
        },
        Field::Id => "id",
        Field::Source => "source",
        Field::Target => "target",
        Field::Weight => "weight",
        Field::SourceHandle => "source_handle",
        Field::TargetHandle => "target_handle",
        _ => return None,
    };
    Some(expr::get_item(name, expr::root()))
}
