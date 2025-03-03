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

        // Member function for handling member.function() syntax
        let member_function = identifier
            .clone()
            .then(
                just('(')
                    .ignore_then(param_list.clone().or_not())
                    .then_ignore(just(')')),
            )
            .map(|(name, params)| Invocation::MemberFunction(name, params.unwrap_or_default()));

        // Invocations
        let invocation = choice((
            identifier
                .clone()
                .map(Invocation::Member)
                .map(Term::Invocation),
            function.map(Term::Invocation),
            member_function.map(Term::Invocation),
            just("$this").to(Term::Invocation(Invocation::This)),
            just("$index").to(Term::Invocation(Invocation::Index)),
            just("$total").to(Term::Invocation(Invocation::Total)),
        ));

        // Terms
        let term = choice((
            invocation.clone(),
            literal,
            external_constant,
            expr.clone()
                .delimited_by(just('('), just(')'))
                .map(|e| Term::Parenthesized(Box::new(e))),
        ));

        // Build the expression parser with operator precedence
        let atom = term.clone().map(Expression::Term);

        // Invocation expression (highest precedence)
        // Support for chained member invocations like Patient.name.given
        let invocation_expr = atom
            .clone()
            .then(
                just('.')
                    .ignore_then(invocation.clone())
                    .map(|inv| match inv {
                        Term::Invocation(i) => i,
                        _ => unreachable!(),
                    })
                    .repeated()
                    .at_least(1),
            )
            .map(|(expr, invs)| {
                invs.into_iter().fold(expr, |acc, inv| {
                    match inv {
                        Invocation::Member(name) => Expression::Invocation(Box::new(acc), name),
                        Invocation::Function(name, _params) => {
                            // Handle function invocation after a dot (member function)
                            Expression::Invocation(Box::new(acc), name)
                        }
                        Invocation::This => unreachable!(),
                        Invocation::Index => unreachable!(),
                        Invocation::Total => unreachable!(),
                        Invocation::MemberFunction(_, _) => unreachable!(),
                    }
                })
            })
            .boxed();

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
        let _function_params = function_param
            .padded() // Allow whitespace around parameters
            .separated_by(just(',').padded()) // Allow whitespace around commas
            .collect::<Vec<_>>();

        // Indexer expression
        let indexer_expr = choice((invocation_expr.clone(), atom.clone()))
            .then(expr.clone().delimited_by(just('['), just(']')).repeated())
            .map(|(expr, indices)| {
                indices.into_iter().fold(expr, |acc, idx| {
                    Expression::Indexer(Box::new(acc), Box::new(idx))
                })
            });

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
                    .then(
                        // First try the parenthesized form
                        just('(')
                            .padded()
                            .ignore_then(qualified_identifier.clone())
                            .then_ignore(just(')').padded())
                            .or(qualified_identifier.clone()) // Then try the simple form
                    )
                    .or_not(),
            )
            .map(|(expr, type_op)| {
                if let Some((op, type_name)) = type_op {
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
