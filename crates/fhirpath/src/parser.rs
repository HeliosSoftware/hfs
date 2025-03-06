use chumsky::Parser;
use chumsky::error::Simple;
use chumsky::prelude::*;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    Boolean(bool),
    String(String),
    Number(f64),
    LongNumber(i64),
    Date(String),
    DateTime(String, String, Option<String>), // date, time, timezone
    Time(String),
    Quantity(f64, Option<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Term(Term),
    Invocation(Box<Expression>, Invocation),
    Indexer(Box<Expression>, Box<Expression>),
    Polarity(char, Box<Expression>),
    Multiplicative(Box<Expression>, String, Box<Expression>),
    Additive(Box<Expression>, String, Box<Expression>),
    Type(Box<Expression>, TypeSpecifier),
    Union(Box<Expression>, Box<Expression>),
    Inequality(Box<Expression>, String, Box<Expression>),
    Equality(Box<Expression>, String, Box<Expression>),
    Membership(Box<Expression>, String, Box<Expression>),
    And(Box<Expression>, Box<Expression>),
    Or(Box<Expression>, String, Box<Expression>),
    Implies(Box<Expression>, Box<Expression>),
    Lambda(Option<String>, Box<Expression>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeSpecifier {
    QualifiedIdentifier(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    Invocation(Invocation),
    Literal(Literal),
    ExternalConstant(String),
    Parenthesized(Box<Expression>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Invocation {
    Member(String),
    Function(String, Vec<Expression>),
    This,
    Index,
    Total,
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Null => write!(f, "{{}}"),
            Literal::Boolean(b) => write!(f, "{}", b),
            Literal::String(s) => write!(f, "'{}'", s),
            Literal::Number(n) => write!(f, "{}", n),
            Literal::LongNumber(n) => write!(f, "{}", n),
            Literal::Date(d) => write!(f, "@{}", d),
            Literal::DateTime(date, time, tz) => {
                write!(f, "@{}T{}", date, time)?;
                if let Some(tz) = tz {
                    write!(f, "{}", tz)?;
                }
                Ok(())
            }
            Literal::Time(t) => write!(f, "@T{}", t),
            Literal::Quantity(n, Some(u)) => write!(f, "{} '{}'", n, u),
            Literal::Quantity(n, None) => write!(f, "{}", n),
        }
    }
}

pub fn parser() -> impl Parser<char, Expression, Error = Simple<char>> {
    // Define the error type we'll use throughout the parser
    type E = Simple<char>;

    // Literals
    let null = just('{').then(just('}')).to(Literal::Null);

    let boolean = text::keyword("true")
        .to(Literal::Boolean(true))
        .or(text::keyword("false").to(Literal::Boolean(false)));

    let string = just('\'')
        .ignore_then(none_of("\'\\").or(just('\\').ignore_then(any())).repeated())
        .then_ignore(just('\''))
        .collect::<String>()
        .map(Literal::String);

    let number = text::int(10)
        .then(just('.').then(text::digits(10)).or_not())
        .map(|(i, d)| {
            if let Some((_, d)) = d {
                Literal::Number(format!("{}.{}", i, d).parse().unwrap())
            } else {
                Literal::Number(i.parse().unwrap())
            }
        })
        .padded(); // Allow whitespace around numbers

    let long_number = text::int(10)
        .then(just('L').or_not())
        .map(|(i, l)| {
            if l.is_some() {
                Literal::LongNumber(i.parse().unwrap())
            } else {
                Literal::LongNumber(i.parse().unwrap())
            }
        })
        .padded(); // Allow whitespace around numbers

    // Date format: YYYY(-MM(-DD)?)?
    // This handles all three valid formats: 1972, 1972-12, 1972-12-14

    // Year only: YYYY (4 digits)
    let year_only = text::digits::<char, E>(10)
        .repeated()
        .exactly(4)
        .collect::<String>();

    // Year and month: YYYY-MM
    let year_month = year_only
        .clone()
        .then(
            just::<char, char, E>('-').ignore_then(
                text::digits::<char, E>(10)
                    .repeated()
                    .exactly(2)
                    .collect::<String>(),
            ),
        )
        .map(|(year, month)| format!("{}-{}", year, month));

    // Full date: YYYY-MM-DD
    let full_date = year_month
        .clone()
        .then(
            just::<char, char, E>('-').ignore_then(
                text::digits::<char, E>(10)
                    .repeated()
                    .exactly(2)
                    .collect::<String>(),
            ),
        )
        .map(|(year_month, day)| format!("{}-{}", year_month, day));

    // Combine all three formats with priority to the most specific match
    let date_format = choice((full_date.clone(), year_month.clone(), year_only.clone()));

    // Time format: HH(:mm(:ss(.sss)?)?)?
    let time_format = text::int::<char, E>(10)
        .then(
            just::<char, char, E>(':')
                .ignore_then(text::int::<char, E>(10))
                .then(
                    just::<char, char, E>(':')
                        .ignore_then(text::int::<char, E>(10))
                        .then(
                            just::<char, char, E>('.')
                                .ignore_then(
                                    text::digits::<char, E>(10)
                                        .repeated()
                                        .at_least(1)
                                        .collect::<String>(),
                                )
                                .or_not(),
                        )
                        .or_not(),
                )
                .or_not(),
        )
        .map(|(hour, min_sec)| {
            let mut result = hour;
            if let Some((min, sec_ms)) = min_sec {
                result.push(':');
                result.push_str(&min);
                if let Some((sec, ms)) = sec_ms {
                    result.push(':');
                    result.push_str(&sec);
                    if let Some(ms) = ms {
                        result.push('.');
                        result.push_str(&ms);
                    }
                }
            }
            result
        });

    // Timezone format: Z | (+|-)HH:mm
    let timezone_format =
        just::<char, char, E>('Z')
            .to("Z".to_string())
            .or(one_of::<char, &str, E>("+-")
                .map(|c: char| c.to_string())
                .then(text::int::<char, E>(10))
                .then(just::<char, char, E>(':'))
                .then(text::int::<char, E>(10))
                .map(|(((sign, hour), _), min)| format!("{}{}:{}", sign, hour, min)));

    // Create a parser for date literals
    let date = just('@')
        .ignore_then(date_format.clone())
        .map(Literal::Date);

    // Create a parser for datetime literals
    let datetime = just('@')
        .ignore_then(date_format.clone())
        .then(
            just('T')
                .ignore_then(time_format.clone())
                .then(timezone_format.or_not()),
        )
        .map(|(date, (time, timezone))| Literal::DateTime(date, time, timezone));

    // Create a parser for time literals
    let time = just('@')
        .then(just('T'))
        .ignore_then(time_format)
        .map(Literal::Time);

    let unit = text::ident().or(just('\'')
        .ignore_then(none_of("\'\\").or(just('\\').ignore_then(any())).repeated())
        .then_ignore(just('\''))
        .collect::<String>());

    let quantity = number.then(unit.or_not()).map(|(n, u)| {
        if let Literal::Number(num) = n {
            Literal::Quantity(num, u)
        } else {
            unreachable!()
        }
    });

    let literal = null
        .or(boolean)
        .or(string)
        .or(number)
        .or(long_number)
        .or(datetime)
        .or(date)
        .or(time)
        .or(quantity)
        .map(Term::Literal);

    // Identifiers
    let identifier = choice((
        text::ident(),
        just('`')
            .ignore_then(none_of("`\\").or(just('\\').ignore_then(any())).repeated())
            .then_ignore(just('`'))
            .collect::<String>(),
        text::keyword("as").to(String::from("as")),
        text::keyword("contains").to(String::from("contains")),
        text::keyword("in").to(String::from("in")),
        text::keyword("is").to(String::from("is")),
    ));

    // Qualified identifier (for type specifiers)
    let qualified_identifier = identifier
        .clone()
        .then(
            just('.')
                .ignore_then(identifier.clone())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(first, rest)| {
            if rest.is_empty() {
                first
            } else {
                let mut result = first;
                for part in rest {
                    result.push_str(".");
                    result.push_str(&part);
                }
                result
            }
        });

    // Create a separate string parser for external constants
    let string_for_external = just('\'')
        .ignore_then(none_of("\'\\").or(just('\\').ignore_then(any())).repeated())
        .then_ignore(just('\''))
        .collect::<String>();

    // External constants
    let external_constant = just('%')
        .ignore_then(choice((identifier.clone(), string_for_external)))
        .map(Term::ExternalConstant);

    let type_specifier = qualified_identifier.map(TypeSpecifier::QualifiedIdentifier);

    // Define operators outside the recursive block
    let multiplicative_op = choice((
        just('*').to("*"),
        just('/').to("/"),
        text::keyword("div").to("div"),
        text::keyword("mod").to("mod"),
    ))
    .padded(); // Allow whitespace around operators

    let additive_op =
        choice((just('+').to("+"), just('-').to("-"), just('&').to("&"))).padded(); // Allow whitespace around operators

    let inequality_op = choice((
        just("<=").to("<="),
        just("<").to("<"),
        just(">=").to(">="),
        just(">").to(">"),
    ))
    .padded(); // Allow whitespace around operators

    let equality_op = choice((
        just("=").to("="),
        just("~").to("~"),
        just("!=").to("!="),
        just("!~").to("!~"),
    ))
    .padded(); // Allow whitespace around operators

    let membership_op = choice((
        text::keyword("in").to("in"),
        text::keyword("contains").to("contains"),
    ))
    .padded(); // Allow whitespace around operators

    let or_op = choice((text::keyword("or").to("or"), text::keyword("xor").to("xor"))).padded(); // Allow whitespace around operators

    // Recursive parser definition
    recursive(|expr| {
        // Function parameters - recursive definition to handle nested expressions
        let param_list = expr.clone().separated_by(just(',')).collect::<Vec<_>>();

        // Function invocation
        let function = identifier
            .clone()
            .then(
                just('(')
                    .ignore_then(param_list.clone().or_not())
                    .then_ignore(just(')')),
            )
            .map(|(name, params)| Invocation::Function(name, params.unwrap_or_default()));

        // Member invocation
        let member_invocation = choice((
            function.clone(),
            identifier.clone().map(Invocation::Member),
            just("$this").to(Invocation::This),
            just("$index").to(Invocation::Index),
            just("$total").to(Invocation::Total),
        ));

        // Term - following the grammar rule for 'term'
        let term = choice((
            member_invocation.clone().map(Term::Invocation),
            literal,
            external_constant,
            expr.clone()
                .delimited_by(just('('), just(')'))
                .map(|e| Term::Parenthesized(Box::new(e))),
        ));

        // Atom expression (basic building block) - maps directly to Term in the grammar
        let atom = term.clone().map(Expression::Term);

        // Invocation chain - handles expression.invocation
        let invocation_chain = atom
            .clone()
            .then(just('.').ignore_then(member_invocation.clone()).repeated())
            .map(|(first, invocations)| {
                invocations.into_iter().fold(first, |expr, invocation| {
                    Expression::Invocation(Box::new(expr), invocation)
                })
            });

        // Indexer expression - handles expression[expression]
        let indexer_expr = invocation_chain
            .clone()
            .then(
                expr.clone()
                    .delimited_by(just('['), just(']'))
                    .map(|idx| (idx,))
                    .repeated(),
            )
            .map(|(first, indices)| {
                indices.into_iter().fold(first, |expr, (idx,)| {
                    Expression::Indexer(Box::new(expr), Box::new(idx))
                })
            });

        // Polarity expression - handles +/- expression
        let polarity_expr = choice((
            just('+')
                .or(just('-'))
                .padded() // Allow whitespace after operator
                .then(indexer_expr.clone())
                .map(|(op, expr)| Expression::Polarity(op, Box::new(expr))),
            indexer_expr.clone(),
        ));

        // Multiplicative expression - handles * / div mod
        let multiplicative_expr = polarity_expr
            .clone()
            .then(multiplicative_op.then(polarity_expr.clone()).repeated())
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, (op, rhs)| {
                    Expression::Multiplicative(Box::new(lhs), op.to_string(), Box::new(rhs))
                })
            });

        // Additive expression - handles + - &
        let additive_expr = multiplicative_expr
            .clone()
            .then(additive_op.then(multiplicative_expr.clone()).repeated())
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, (op, rhs)| {
                    Expression::Additive(Box::new(lhs), op.to_string(), Box::new(rhs))
                })
            });

        // Type expression - handles 'is' and 'as'
        let type_expr = additive_expr
            .clone()
            .then(
                choice((just("is"), just("as")))
                    .padded() // Allow whitespace around 'is' and 'as'
                    .then(type_specifier.clone())
                    .or_not(),
            )
            .map(|(expr, type_op)| {
                if let Some((op, type_name)) = type_op {
                    Expression::Type(Box::new(expr), type_name)
                } else {
                    expr
                }
            });

        // Union expression - handles |
        let union_expr = type_expr
            .clone()
            .then(just('|').padded().ignore_then(type_expr.clone()).or_not()) // Allow whitespace around '|'
            .map(|(first, rest)| {
                if let Some(rest) = rest {
                    Expression::Union(Box::new(first), Box::new(rest))
                } else {
                    first
                }
            });

        // Inequality expression - handles <= < > >=
        let inequality_expr = union_expr
            .clone()
            .then(inequality_op.then(union_expr.clone()).or_not())
            .map(|(lhs, rhs)| {
                if let Some((op, rhs)) = rhs {
                    Expression::Inequality(Box::new(lhs), op.to_string(), Box::new(rhs))
                } else {
                    lhs
                }
            });

        // Equality expression - handles = ~ != !~
        let equality_expr = inequality_expr
            .clone()
            .then(equality_op.then(inequality_expr.clone()).or_not())
            .map(|(lhs, rhs)| {
                if let Some((op, rhs)) = rhs {
                    Expression::Equality(Box::new(lhs), op.to_string(), Box::new(rhs))
                } else {
                    lhs
                }
            });

        // Membership expression - handles 'in' and 'contains'
        let membership_expr = equality_expr
            .clone()
            .then(membership_op.then(equality_expr.clone()).or_not())
            .map(|(lhs, rhs)| {
                if let Some((op, rhs)) = rhs {
                    Expression::Membership(Box::new(lhs), op.to_string(), Box::new(rhs))
                } else {
                    lhs
                }
            });

        // And expression - handles 'and'
        let and_expr = membership_expr
            .clone()
            .then(
                text::keyword("and")
                    .padded()
                    .ignore_then(membership_expr.clone())
                    .repeated(),
            ) // Allow whitespace around 'and'
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, rhs| {
                    Expression::And(Box::new(lhs), Box::new(rhs))
                })
            });

        // Or expression - handles 'or' and 'xor'
        let or_expr = and_expr
            .clone()
            .then(or_op.then(and_expr.clone()).repeated())
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, (op, rhs)| {
                    Expression::Or(Box::new(lhs), op.to_string(), Box::new(rhs))
                })
            });

        // Implies expression - handles 'implies'
        let implies_expr = or_expr
            .clone()
            .then(
                text::keyword("implies")
                    .padded()
                    .ignore_then(or_expr.clone())
                    .or_not(),
            ) // Allow whitespace around 'implies'
            .map(|(lhs, rhs)| {
                if let Some(rhs) = rhs {
                    Expression::Implies(Box::new(lhs), Box::new(rhs))
                } else {
                    lhs
                }
            });

        // Return the final parser
        implies_expr
    })
    .then_ignore(end())
}
