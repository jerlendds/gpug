use super::*;

const MAX_BYTES: usize = 65_536;
const MAX_TOKENS: usize = 2048;
const MAX_DEPTH: usize = 32;

#[derive(Debug, PartialEq, Eq)]
enum TokenKind {
    Word(String),
    Text(String),
    Symbol(String),
}
#[derive(Debug)]
struct Token {
    offset: usize,
    kind: TokenKind,
}
struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    end: usize,
    source: QuerySource,
    program: Vec<Instruction>,
}

fn error(offset: usize, message: impl Into<String>) -> QueryError {
    QueryError {
        offset,
        message: message.into(),
    }
}

pub(super) fn parse(text: &str) -> Result<Query, QueryError> {
    if text.len() > MAX_BYTES {
        return Err(error(MAX_BYTES, "query exceeds 65536 bytes"));
    }
    let mut parser = Parser {
        tokens: lex(text)?,
        cursor: 0,
        end: text.len(),
        source: QuerySource::Nodes,
        program: Vec::new(),
    };
    parser.source = if parser.eat("NODES") {
        QuerySource::Nodes
    } else if parser.eat("EDGES") {
        QuerySource::Edges
    } else if parser.eat("NEIGHBORS") {
        let seed = NodeId(parser.unsigned()?);
        let direction = if parser.eat("OUTGOING") {
            Direction::Outgoing
        } else if parser.eat("INCOMING") {
            Direction::Incoming
        } else if parser.eat("BOTH") {
            Direction::Both
        } else {
            return Err(parser.error("expected OUTGOING, INCOMING or BOTH"));
        };
        parser.require("HOPS")?;
        let hops = parser.size()?;
        QuerySource::Neighbors {
            seed,
            direction,
            hops,
        }
    } else {
        return Err(parser.error("expected NODES, EDGES or NEIGHBORS"));
    };
    if parser.eat("WHERE") {
        parser.or(0)?;
    }
    let limit = if parser.eat("LIMIT") {
        Some(parser.size()?)
    } else {
        None
    };
    parser.eat(";");
    if parser.cursor != parser.tokens.len() {
        return Err(parser.error("unexpected trailing token"));
    }
    Ok(Query {
        source: parser.source,
        program: parser.program,
        limit,
    })
}

impl Parser {
    fn offset(&self) -> usize {
        self.tokens.get(self.cursor).map_or(self.end, |t| t.offset)
    }
    fn error(&self, message: impl Into<String>) -> QueryError {
        error(self.offset(), message)
    }
    fn eat(&mut self, expected: &str) -> bool {
        let matches = self
            .tokens
            .get(self.cursor)
            .is_some_and(|token| match &token.kind {
                TokenKind::Word(word) | TokenKind::Symbol(word) => {
                    word.eq_ignore_ascii_case(expected)
                }
                TokenKind::Text(_) => false,
            });
        if matches {
            self.cursor += 1;
        }
        matches
    }
    fn require(&mut self, expected: &str) -> Result<(), QueryError> {
        if self.eat(expected) {
            Ok(())
        } else {
            Err(self.error(format!("expected {expected}")))
        }
    }
    fn word(&mut self) -> Result<(usize, String), QueryError> {
        match self.tokens.get(self.cursor) {
            Some(Token {
                offset,
                kind: TokenKind::Word(word),
            }) => {
                let result = (*offset, word.clone());
                self.cursor += 1;
                Ok(result)
            }
            _ => Err(self.error("expected identifier or number")),
        }
    }
    fn unsigned(&mut self) -> Result<u64, QueryError> {
        let (offset, word) = self.word()?;
        if !word.bytes().all(|b| b.is_ascii_digit()) {
            return Err(error(offset, "expected unsigned decimal integer"));
        }
        word.parse()
            .map_err(|_| error(offset, "integer exceeds u64 range"))
    }
    fn size(&mut self) -> Result<usize, QueryError> {
        let offset = self.offset();
        usize::try_from(self.unsigned()?)
            .map_err(|_| error(offset, "integer exceeds platform size range"))
    }
    fn or(&mut self, depth: usize) -> Result<(), QueryError> {
        self.and(depth)?;
        while self.eat("OR") {
            self.and(depth)?;
            self.program.push(Instruction::Or);
        }
        Ok(())
    }
    fn and(&mut self, depth: usize) -> Result<(), QueryError> {
        self.unary(depth)?;
        while self.eat("AND") {
            self.unary(depth)?;
            self.program.push(Instruction::And);
        }
        Ok(())
    }
    fn unary(&mut self, depth: usize) -> Result<(), QueryError> {
        if depth > MAX_DEPTH {
            return Err(self.error("predicate nesting exceeds 32 levels"));
        }
        if self.eat("NOT") {
            self.unary(depth + 1)?;
            self.program.push(Instruction::Not);
        } else if self.eat("(") {
            self.or(depth + 1)?;
            self.require(")")?;
        } else {
            self.comparison()?;
        }
        Ok(())
    }
    fn comparison(&mut self) -> Result<(), QueryError> {
        let (offset, name) = self.word()?;
        let field = field(&name, self.source).ok_or_else(|| {
            error(
                offset,
                format!("unknown field {name} for this query source"),
            )
        })?;
        if self.eat("IS") {
            let negate = self.eat("NOT");
            self.require("NULL")?;
            self.program.push(Instruction::IsNull(field));
            if negate {
                self.program.push(Instruction::Not);
            }
            return Ok(());
        }
        let offset = self.offset();
        let op = if self.eat("=") {
            Op::Eq
        } else if self.eat("!=") || self.eat("<>") {
            Op::Ne
        } else if self.eat("<") {
            Op::Lt
        } else if self.eat("<=") {
            Op::Le
        } else if self.eat(">") {
            Op::Gt
        } else if self.eat(">=") {
            Op::Ge
        } else if self.eat("CONTAINS") {
            Op::Contains
        } else {
            return Err(self.error("expected comparison operator"));
        };
        let kind = field.kind();
        if (op == Op::Contains && kind != Kind::Text)
            || (kind == Kind::Bool && op != Op::Eq && op != Op::Ne)
        {
            return Err(error(
                offset,
                "operator is not supported for this field type",
            ));
        }
        let offset = self.offset();
        let literal = match kind {
            Kind::U64 => Literal::U64(self.unsigned()?),
            Kind::Number => {
                let (_, word) = self.word()?;
                let number: f64 = word
                    .parse()
                    .map_err(|_| error(offset, "expected finite numeric literal"))?;
                if !number.is_finite() {
                    return Err(error(offset, "expected finite numeric literal"));
                }
                Literal::Number(number)
            }
            Kind::Text => match self.tokens.get(self.cursor) {
                Some(Token {
                    kind: TokenKind::Text(text),
                    ..
                }) => {
                    let text = text.clone();
                    self.cursor += 1;
                    Literal::Text(text)
                }
                _ => return Err(self.error("expected single-quoted string")),
            },
            Kind::Bool => {
                if self.eat("TRUE") {
                    Literal::Bool(true)
                } else if self.eat("FALSE") {
                    Literal::Bool(false)
                } else {
                    return Err(self.error("expected TRUE or FALSE"));
                }
            }
        };
        self.program.push(Instruction::Compare(field, op, literal));
        Ok(())
    }
}

fn node_field(name: &str) -> Option<NodeField> {
    Some(match name {
        "id" => NodeField::Id,
        "label" => NodeField::Label,
        "renderer" => NodeField::Renderer,
        "hidden" => NodeField::Hidden,
        "x" => NodeField::X,
        "y" => NodeField::Y,
        "width" => NodeField::Width,
        "height" => NodeField::Height,
        "in_degree" => NodeField::InDegree,
        "out_degree" => NodeField::OutDegree,
        _ => return None,
    })
}
fn field(name: &str, source: QuerySource) -> Option<Field> {
    let name = name.to_ascii_lowercase();
    if source != QuerySource::Edges {
        return node_field(&name).map(Field::Node);
    }
    if let Some(name) = name.strip_prefix("source.") {
        return node_field(name).map(Field::SourceNode);
    }
    if let Some(name) = name.strip_prefix("target.") {
        return node_field(name).map(Field::TargetNode);
    }
    Some(match name.as_str() {
        "id" => Field::Id,
        "source" => Field::Source,
        "target" => Field::Target,
        "source_handle" => Field::SourceHandle,
        "target_handle" => Field::TargetHandle,
        "weight" => Field::Weight,
        _ => return None,
    })
}

fn lex(text: &str) -> Result<Vec<Token>, QueryError> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if tokens.len() == MAX_TOKENS {
            return Err(error(offset, "query exceeds 2048 tokens"));
        }
        let kind = if ch == '\'' {
            let mut value = String::new();
            loop {
                let Some((_, ch)) = chars.next() else {
                    return Err(error(offset, "unterminated string"));
                };
                if ch == '\'' {
                    if chars.peek().is_some_and(|(_, c)| *c == '\'') {
                        chars.next();
                        value.push('\'');
                    } else {
                        break;
                    }
                } else {
                    value.push(ch);
                }
            }
            TokenKind::Text(value)
        } else if "()=<>!;".contains(ch) {
            let mut symbol = ch.to_string();
            if chars
                .peek()
                .is_some_and(|(_, c)| ("<>!".contains(ch) && *c == '=') || (ch == '<' && *c == '>'))
            {
                symbol.push(chars.next().expect("peeked character").1);
            }
            TokenKind::Symbol(symbol)
        } else {
            let mut word = ch.to_string();
            while chars
                .peek()
                .is_some_and(|(_, c)| !c.is_whitespace() && !"'()=<>!;".contains(*c))
            {
                word.push(chars.next().expect("peeked character").1);
            }
            TokenKind::Word(word)
        };
        tokens.push(Token { offset, kind });
    }
    Ok(tokens)
}
