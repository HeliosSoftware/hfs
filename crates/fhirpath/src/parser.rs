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
    DateTime(String),
    Time(String),
    Quantity(f64, Option<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Term(Term),
    Invocation(Box<Expression>, String),
    Indexer(Box<Expression>, Box<Expression>),
    Polarity(char, Box<Expression>),
    Multiplicative(Box<Expression>, String, Box<Expression>),
    Additive(Box<Expression>, String, Box<Expression>),
    Type(Box<Expression>, String),
    Union(Box<Expression>, Box<Expression>),
    Inequality(Box<Expression>, String, Box<Expression>),
    Equality(Box<Expression>, String, Box<Expression>),
    Membership(Box<Expression>, String, Box<Expression>),
    And(Box<Expression>, Box<Expression>),
    Or(Box<Expression>, String, Box<Expression>),
    Implies(Box<Expression>, Box<Expression>),
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
    MemberFunction(String, Vec<Expression>), // For member.function() syntax
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
            Literal::DateTime(dt) => write!(f, "@{}", dt),
            Literal::Time(t) => write!(f, "@T{}", t),
            Literal::Quantity(n, Some(u)) => write!(f, "{} '{}'", n, u),
            Literal::Quantity(n, None) => write!(f, "{}", n),
        }
    }
}

pub fn parser() -> impl Parser<char, Expression, Error = Simple<char>> {
    // Define the error type we'll use throughout the parser
    type E = Simple<char>;
    // Recursive parser definition with explicit recursion limit
    recursive(|expr| {
        let expr = expr.boxed();
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

        // Add longnumber, which is LONGNUMBER in the grammar AI!

        // Date format: YYYY(-MM(-DD))?
        let _date_format = text::int::<char, E>(10)
            .map(|s: String| s)
            .then(
                just::<char, char, E>('-')
                    .ignore_then(text::int::<char, E>(10))
                    .then(
                        just::<char, char, E>('-')
                            .ignore_then(text::int::<char, E>(10))
                            .or_not(),
                    )
                    .or_not(),
            )
            .map(|(year, month_day)| {
                let mut result = year;
                if let Some((month, day)) = month_day {
                    result.push('-');
                    result.push_str(&month);
                    if let Some(day) = day {
                        result.push('-');
                        result.push_str(&day);
                    }
                }
                result
            });

        // Time format: HH(:mm(:ss(.sss)?)?)?
        let _time_format = text::int::<char, E>(10)
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
        let _timezone_format =
            just::<char, char, E>('Z')
                .to("Z".to_string())
                .or(one_of::<char, &str, E>("+-")
                    .map(|c: char| c.to_string())
                    .then(text::int::<char, E>(10))
                    .then(just::<char, char, E>(':'))
                    .then(text::int::<char, E>(10))
                    .map(|(((sign, hour), _), min)| format!("{}{}:{}", sign, hour, min)));

        // Create a parser for date literals that captures the entire date string
        let date_literal = just::<char, char, E>('@')
            .ignore_then(
                filter(|c: &char| c.is_digit(10) || *c == '-')
                    .repeated()
                    .at_least(1)
                    .collect::<String>(),
            )
            .map(Literal::Date)
            .boxed()
            .then(
                just('.')
                    .ignore_then(
                        just("is")
                            .padded()
                            .then(just("Date"))
                            .then(just('(').ignore_then(just(')'))),
                    )
                    .or_not(),
            )
            .map(|(date, is_check)| {
                if is_check.is_some() {
                    Expression::Type(
                        Box::new(Expression::Term(Term::Literal(date))),
                        "Date".to_string(),
                    )
                } else {
                    Expression::Term(Term::Literal(date))
                }
            })
            .boxed();

        // Create a parser for datetime literals that captures the entire datetime string
        let datetime_literal = just::<char, char, E>('@')
            .ignore_then(
                filter(|c: &char| c.is_digit(10) || *c == '-')
                    .repeated()
                    .at_least(1)
                    .collect::<String>()
                    .then(
                        just::<char, char, E>('T')
                            .ignore_then(
                                filter(|c: &char| {
                                    c.is_digit(10)
                                        || *c == ':'
                                        || *c == '.'
                                        || *c == '+'
                                        || *c == '-'
                                        || *c == 'Z'
                                })
                                .repeated()
                                .at_least(1)
                                .collect::<String>(),
                            )
                            .map(|t| format!("T{}", t)),
                    )
                    .map(|(d, t)| format!("{}{}", d, t)),
            )
            .map(Literal::DateTime);

        // Create a parser for time literals that captures the entire time string
        let time_literal = just::<char, char, E>('@')
            .then(just::<char, char, E>('T'))
            .ignore_then(
                filter(|c: &char| {
                    c.is_digit(10) || *c == ':' || *c == '.' || *c == '+' || *c == '-' || *c == 'Z'
                })
                .repeated()
                .at_least(1)
                .collect::<String>(),
            )
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
            .or(datetime_literal)
            .or(date_literal)
            .or(time_literal)
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

        // Function parameters
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

        // Invocations
        let invocation = choice((
            function.map(Term::Invocation),
            just("$this").to(Term::Invocation(Invocation::This)),
            just("$index").to(Term::Invocation(Invocation::Index)),
            just("$total").to(Term::Invocation(Invocation::Total)),
            identifier
                .clone()
                .map(|id| Term::Invocation(Invocation::Member(id))),
        ));

        // Terms
        let term = choice((
            invocation,
            literal,
            external_constant,
            expr.clone()
                .delimited_by(just('('), just(')'))
                .map(|e| Term::Parenthesized(Box::new(e))),
        ));

        // Build the expression parser with operator precedence
        let atom = term.clone().map(Expression::Term);

        // Function call parameter parser - handles expressions inside function calls
        let function_param = recursive(|_| {
            // Create a specialized parser for function parameters that handles equality expressions
            let param_atom = term.clone().map(Expression::Term);

            let param_member = param_atom
                .then(just('.').ignore_then(identifier.clone()).repeated())
                .map(|(expr, members)| {
                    members.into_iter().fold(expr, |acc, member| {
                        Expression::Invocation(Box::new(acc), member)
                    })
                });

            // Handle equality expressions in parameters
            let param_equality = param_member
                .clone()
                .then(
                    just('=')
                        .padded() // Allow whitespace around equals sign
                        .to("=")
                        .then(
                            just('\'')
                                .ignore_then(
                                    none_of("\'\\").or(just('\\').ignore_then(any())).repeated(),
                                )
                                .then_ignore(just('\''))
                                .collect::<String>()
                                .map(|s| Expression::Term(Term::Literal(Literal::String(s)))),
                        )
                        .or_not(),
                )
                .map(|(expr, eq_opt)| {
                    if let Some((op, rhs)) = eq_opt {
                        Expression::Equality(Box::new(expr), op.to_string(), Box::new(rhs))
                    } else {
                        expr
                    }
                });

            param_equality
        });

        // Function parameters list
        let function_params = function_param
            .padded() // Allow whitespace around parameters
            .separated_by(just(',').padded()) // Allow whitespace around commas
            .collect::<Vec<_>>();

        // Invocation expression (highest precedence)
        let invocation_expr = atom
            .then(
                just('.')
                    .ignore_then(
                        // Either a function call or a simple member access
                        identifier.clone().then(
                            just('(')
                                .ignore_then(function_params.padded().or_not()) // Allow whitespace inside parentheses
                                .then_ignore(just(')'))
                                .or_not(),
                        ),
                    )
                    .repeated(),
            )
            .map(|(expr, invocations)| {
                invocations
                    .into_iter()
                    .fold(expr, |acc, (name, params_opt)| {
                        if let Some(_params) = params_opt {
                            // It's a function call with parameters
                            Expression::Invocation(Box::new(acc), format!("{}()", name))
                        } else {
                            // It's a simple member access
                            Expression::Invocation(Box::new(acc), name)
                        }
                    })
            });

        // Special case for date/time literals with method calls
        let date_time_method = choice((
            datetime_literal
                .clone()
                .map(Term::Literal)
                .map(Expression::Term)
                .then(
                    just('.')
                        .ignore_then(
                            just("is")
                                .padded()
                                .then(just("DateTime").to("DateTime".to_string()))
                                .then(just('(').ignore_then(just(')'))),
                        )
                        .or_not(),
                )
                .map(|(expr, method_opt)| {
                    if let Some((method_type, _)) = method_opt {
                        let (_, type_name) = method_type;
                        Expression::Type(Box::new(expr), type_name)
                    } else {
                        expr
                    }
                }),
            time_literal
                .clone()
                .map(Term::Literal)
                .map(Expression::Term)
                .then(
                    just('.')
                        .ignore_then(
                            just("is")
                                .padded()
                                .then(just("Time").to("Time".to_string()))
                                .then(just('(').ignore_then(just(')'))),
                        )
                        .or_not(),
                )
                .map(|(expr, method_opt)| {
                    if let Some((method_type, _)) = method_opt {
                        let (_, type_name) = method_type;
                        Expression::Type(Box::new(expr), type_name)
                    } else {
                        expr
                    }
                }),
            date_literal.clone(),
        ))
        .boxed();

        // Indexer expression
        let indexer_expr = choice((
            date_time_method,
            invocation_expr
                .then(expr.clone().delimited_by(just('['), just(']')).repeated())
                .map(|(expr, indices)| {
                    indices.into_iter().fold(expr, |acc, idx| {
                        Expression::Indexer(Box::new(acc), Box::new(idx))
                    })
                }),
        ));

        // Polarity expression
        let polarity_expr = choice((
            just('+')
                .or(just('-'))
                .padded() // Allow whitespace after operator
                .then(indexer_expr.clone())
                .map(|(op, expr)| Expression::Polarity(op, Box::new(expr))),
            indexer_expr.clone(),
        ))
        .boxed();

        // Multiplicative expression
        let op1 = choice((
            just('*').to("*"),
            just('/').to("/"),
            text::keyword("div").to("div"),
            text::keyword("mod").to("mod"),
        ))
        .padded(); // Allow whitespace around operators

        let multiplicative_expr = polarity_expr
            .clone()
            .then(op1.then(polarity_expr.clone()).repeated())
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, (op, rhs)| {
                    Expression::Multiplicative(Box::new(lhs), op.to_string(), Box::new(rhs))
                })
            })
            .boxed();

        // Additive expression
        let op2 = choice((just('+').to("+"), just('-').to("-"), just('&').to("&"))).padded(); // Allow whitespace around operators

        let additive_expr = multiplicative_expr
            .clone()
            .then(op2.then(multiplicative_expr.clone()).repeated())
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, (op, rhs)| {
                    Expression::Additive(Box::new(lhs), op.to_string(), Box::new(rhs))
                })
            })
            .boxed();

        // Type expression
        let type_expr = additive_expr
            .clone()
            .then(
                choice((just("is"), just("as")))
                    .padded() // Allow whitespace around 'is' and 'as'
                    .then(qualified_identifier)
                    .or_not(),
            )
            .map(|(expr, type_op)| {
                if let Some((_op, type_name)) = type_op {
                    Expression::Type(Box::new(expr), type_name)
                } else {
                    expr
                }
            })
            .boxed();

        // Union expression
        let union_expr = type_expr
            .clone()
            .then(just('|').padded().ignore_then(type_expr.clone()).repeated()) // Allow whitespace around '|'
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, rhs| {
                    Expression::Union(Box::new(lhs), Box::new(rhs))
                })
            })
            .boxed();

        // Inequality expression
        let op3 = choice((
            just("<=").to("<="),
            just("<").to("<"),
            just(">=").to(">="),
            just(">").to(">"),
        ))
        .padded(); // Allow whitespace around operators

        let inequality_expr = union_expr
            .clone()
            .then(op3.then(union_expr.clone()).or_not())
            .map(|(lhs, rhs)| {
                if let Some((op, rhs)) = rhs {
                    Expression::Inequality(Box::new(lhs), op.to_string(), Box::new(rhs))
                } else {
                    lhs
                }
            })
            .boxed();

        // Equality expression
        let op4 = choice((
            just("=").to("="),
            just("~").to("~"),
            just("!=").to("!="),
            just("!~").to("!~"),
        ))
        .padded(); // Allow whitespace around operators

        let equality_expr = inequality_expr
            .clone()
            .then(op4.then(inequality_expr.clone()).or_not())
            .map(|(lhs, rhs)| {
                if let Some((op, rhs)) = rhs {
                    Expression::Equality(Box::new(lhs), op.to_string(), Box::new(rhs))
                } else {
                    lhs
                }
            })
            .boxed();

        // Membership expression
        let op5 = choice((
            text::keyword("in").to("in"),
            text::keyword("contains").to("contains"),
        ))
        .padded(); // Allow whitespace around operators

        let membership_expr = equality_expr
            .clone()
            .then(op5.then(equality_expr).or_not())
            .map(|(lhs, rhs)| {
                if let Some((op, rhs)) = rhs {
                    Expression::Membership(Box::new(lhs), op.to_string(), Box::new(rhs))
                } else {
                    lhs
                }
            })
            .boxed();

        // And expression
        let and_expr = membership_expr
            .clone()
            .then(
                text::keyword("and")
                    .padded()
                    .ignore_then(membership_expr)
                    .repeated(),
            ) // Allow whitespace around 'and'
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, rhs| {
                    Expression::And(Box::new(lhs), Box::new(rhs))
                })
            })
            .boxed();

        // Or expression
        let op6 = choice((text::keyword("or").to("or"), text::keyword("xor").to("xor"))).padded(); // Allow whitespace around operators

        let or_expr = and_expr
            .clone()
            .then(op6.then(and_expr).repeated())
            .map(|(first, rest)| {
                rest.into_iter().fold(first, |lhs, (op, rhs)| {
                    Expression::Or(Box::new(lhs), op.to_string(), Box::new(rhs))
                })
            })
            .boxed();

        // Implies expression
        let implies_expr = or_expr
            .clone()
            .then(
                text::keyword("implies")
                    .padded()
                    .ignore_then(or_expr)
                    .or_not(),
            ) // Allow whitespace around 'implies'
            .map(|(lhs, rhs)| {
                if let Some(rhs) = rhs {
                    Expression::Implies(Box::new(lhs), Box::new(rhs))
                } else {
                    lhs
                }
            })
            .boxed();

        implies_expr
    })
    .then_ignore(end())
}
