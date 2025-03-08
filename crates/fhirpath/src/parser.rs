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

pub fn parser() -> impl Parser<char, Expression, Error = Simple<char>> + Clone {
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

    // Time format: HH(:mm(:ss(.sss)?)?)?
    let time_format = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(2)
        .at_most(2)
        .collect::<String>()
        .then(
            just(':')
                .ignore_then(
                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                        .repeated()
                        .at_least(2)
                        .at_most(2)
                        .collect::<String>(),
                )
                .then(
                    just(':')
                        .ignore_then(
                            filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                                .repeated()
                                .at_least(2)
                                .at_most(2)
                                .collect::<String>(),
                        )
                        .then(
                            just('.').ignore_then(
                                filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                                    .repeated()
                                    .at_least(1)
                                    .at_most(3)
                                    .collect::<String>()
                                    .or_not(),
                            ),
                        )
                        .or_not(),
                )
                .or_not(),
        )
            // AI! Add correct map
        });

    // Timezone format: Z | (+|-)HH:mm
    let timezone_format = just('Z').to("Z".to_string()).or(one_of("+-")
        .map(|c: char| c.to_string())
        .then(
            text::digits(10)
                .repeated()
                .at_most(2)
                .at_least(2)
                .collect::<String>(),
        )
        .then(just(':'))
        .then(
            text::digits(10)
                .repeated()
                .at_most(2)
                .at_least(2)
                .collect::<String>(),
        )
        .map(|(((sign, hour), _), min)| format!("{}{}:{}", sign, hour, min)));

    // Date format: YYYY(-MM(-DD)?)?
    // This handles all valid formats: 1972, 2015, 1972-12, 1972-12-14
    let date_format = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .exactly(4)
        .collect::<String>()
        .then(
            just('-')
                .ignore_then(
                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                        .repeated()
                        .exactly(2)
                        .collect::<String>()
                        .then(
                            just('-')
                                .ignore_then(
                                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                                        .repeated()
                                        .exactly(2)
                                        .collect::<String>(),
                                )
                                .or_not(),
                        ),
                )
                .or_not(),
        )
        .map(|(year, month_part)| {
            let mut date_str = year;

            // month_part is Option<(month_str, Option<day_str>)>
            if let Some((month_str, day_part)) = month_part {
                date_str.push('-');
                date_str.push_str(&month_str);

                // day_part is Option<day_str>
                if let Some(day_str) = day_part {
                    date_str.push('-');
                    date_str.push_str(&day_str);
                }
            }

            println!("Parsed date: {}", date_str);

            Literal::Date(date_str)
        })
        .boxed();

    let date = just('@').ignore_then(date_format.clone());

    // Create a parser for datetime literals
    let datetime = just('@')
        .ignore_then(date_format)
        .then(
            just('T')
                .ignore_then(time_format.then(timezone_format.or_not()))
                .or_not(),
        )
        .map(|(date_lit, time_opt)| {
            if let Literal::Date(date_str) = date_lit {
                if let Some((time_str, tz_opt)) = time_opt {
                    Literal::DateTime(date_str, time_str, tz_opt)
                } else {
                    // If no time part is provided, use empty string for time
                    Literal::DateTime(date_str, "".to_string(), None)
                }
            } else {
                unreachable!("Expected Date literal")
            }
        })
        .boxed();

    // Create a parser for time literals
    let time = just('@')
        .ignore_then(just('T').ignore_then(time_format.clone()))
        .map(|time_str| Literal::Time(time_str))
        .boxed();

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

    let literal = choice((
        null,
        boolean,
        string,
        number,
        long_number,
        date,
        datetime,
        time.clone(),
        quantity,
    ))
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
        })
        .boxed(); // Box the parser to make it easier to clone

    // Create a separate string parser for external constants
    let string_for_external = just('\'')
        .ignore_then(none_of("\'\\").or(just('\\').ignore_then(any())).repeated())
        .then_ignore(just('\''))
        .collect::<String>();

    // External constants
    let external_constant = just('%')
        .ignore_then(choice((identifier.clone(), string_for_external)))
        .map(Term::ExternalConstant);

    let type_specifier = qualified_identifier
        .clone()
        .map(TypeSpecifier::QualifiedIdentifier);

    // Define operators outside the recursive block
    let multiplicative_op = choice((
        just('*').to("*"),
        just('/').to("/"),
        text::keyword("div").to("div"),
        text::keyword("mod").to("mod"),
    ))
    .padded(); // Allow whitespace around operators

    let additive_op = choice((just('+').to("+"), just('-').to("-"), just('&').to("&"))).padded(); // Allow whitespace around operators

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

    // Recursive parser definition - limit recursion depth to prevent stack overflow
    let expr_parser = recursive(|expr| {
        // Function parameters - recursive definition to handle nested expressions
        let param_list = expr
            .clone()
            .separated_by(just(',').padded())
            .collect::<Vec<_>>()
            .boxed();

        // Function invocation
        let function = identifier
            .clone()
            .then(
                just('(')
                    .ignore_then(param_list.clone().or_not().map(|p| p.unwrap_or_default()))
                    .then_ignore(just(')')),
            )
            .map(|(name, params)| Invocation::Function(name, params))
            .boxed();

        // Member invocation
        let member_invocation = choice((
            function.clone(),
            identifier.clone().map(Invocation::Member),
            just("$this").to(Invocation::This),
            just("$index").to(Invocation::Index),
            just("$total").to(Invocation::Total),
        ))
        .boxed();

        // Term - following the grammar rule for 'term'
        let term = choice((
            member_invocation.clone().map(Term::Invocation),
            literal,
            external_constant,
            expr.clone()
                .delimited_by(just('('), just(')'))
                .map(|e| Term::Parenthesized(Box::new(e))),
        ))
        .boxed();

        // Atom expression (basic building block) - maps directly to Term in the grammar
        let atom = term.clone().map(Expression::Term);

        // Invocation chain - handles expression.invocation
        // This needs to handle function calls including 'is' as a function name
        let invocation_chain = atom
            .clone()
            .then(just('.').ignore_then(member_invocation.clone()).repeated())
            .map(|(first, invocations)| {
                invocations.into_iter().fold(first, |expr, invocation| {
                    Expression::Invocation(Box::new(expr), invocation)
                })
            })
            .boxed();

        // Indexer expression - handles expression[expression]
        let indexer_expr = invocation_chain
            .clone()
            .then(
                expr.clone()
                    .delimited_by(just('['), just(']'))
                    .map(|idx| idx)
                    .repeated(),
            )
            .map(|(first, indices)| {
                indices.into_iter().fold(first, |expr, idx| {
                    Expression::Indexer(Box::new(expr), Box::new(idx))
                })
            })
            .boxed();

        // Polarity expression - handles +/- expression
        let polarity_expr = choice((
            just('+')
                .or(just('-'))
                .padded() // Allow whitespace after operator
                .then(indexer_expr.clone())
                .map(|(op, expr)| Expression::Polarity(op, Box::new(expr))),
            indexer_expr.clone(),
        ))
        .boxed();

        // Multiplicative expression - handles * / div mod
        let multiplicative_expr = polarity_expr
            .clone()
            .then(multiplicative_op.then(polarity_expr.clone()).repeated())
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, (op, rhs)| {
                    Expression::Multiplicative(Box::new(lhs), op.to_string(), Box::new(rhs))
                })
            })
            .boxed();

        // Additive expression - handles + - &
        let additive_expr = multiplicative_expr
            .clone()
            .then(additive_op.then(multiplicative_expr.clone()).repeated())
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, (op, rhs)| {
                    Expression::Additive(Box::new(lhs), op.to_string(), Box::new(rhs))
                })
            })
            .boxed();

        // Type expression - handles 'is' and 'as' as operators
        // We need to be careful not to confuse this with function calls
        let type_expr = additive_expr
            .clone()
            .then(
                // Handle is/as followed by a type name
                choice((
                    just("is").padded().map(|_| "is"),
                    just("as").padded().map(|_| "as"),
                ))
                .then(type_specifier.clone())
                .or_not(),
            )
            .map(|(expr, op_type)| {
                if let Some((_, type_specifier)) = op_type {
                    Expression::Type(Box::new(expr), type_specifier)
                } else {
                    expr
                }
            })
            .boxed();

        // Union expression - handles |
        let union_expr = type_expr
            .clone()
            .then(just('|').padded().ignore_then(type_expr.clone()).repeated()) // Allow whitespace around '|'
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |acc, next| {
                    Expression::Union(Box::new(acc), Box::new(next))
                })
            })
            .boxed();

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
            })
            .boxed();

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
            })
            .boxed();

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
            })
            .boxed();

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

        implies_expr
    });

    // Return the parser
    expr_parser.then_ignore(end())
}
