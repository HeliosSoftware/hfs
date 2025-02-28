use chumsky::prelude::*;
use chumsky::Parser;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    Boolean(bool),
    String(String),
    Number(f64),
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
            Literal::Date(d) => write!(f, "@{}", d),
            Literal::DateTime(dt) => write!(f, "@{}", dt),
            Literal::Time(t) => write!(f, "@T{}", t),
            Literal::Quantity(n, Some(u)) => write!(f, "{} '{}'", n, u),
            Literal::Quantity(n, None) => write!(f, "{}", n),
        }
    }
}

pub fn parser() -> impl Parser<char, Expression, Error = Simple<char>> {
    // Recursive parser definition
    recursive(|expr| {
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
            });

        let date = just('@')
            .ignore_then(
                filter(|c: &char| c.is_ascii_digit() || *c == '-')
                    .repeated()
                    .at_least(4)
                    .collect::<String>(),
            )
            .map(Literal::Date);

        let datetime = just('@')
            .ignore_then(
                filter(|c: &char| {
                    c.is_ascii_digit()
                        || *c == '-'
                        || *c == 'T'
                        || *c == ':'
                        || *c == '.'
                        || *c == 'Z'
                        || *c == '+'
                })
                .repeated()
                .collect::<String>(),
            )
            .map(Literal::DateTime);

        let time = just('@')
            .then(just('T'))
            .ignore_then(
                filter(|c: &char| c.is_ascii_digit() || *c == ':' || *c == '.')
                    .repeated()
                    .collect::<String>(),
            )
            .map(Literal::Time);

        let unit = text::ident().or(just('\'')
            .ignore_then(none_of("\'\\").or(just('\\').ignore_then(any())).repeated())
            .then_ignore(just('\''))
            .collect::<String>());

        let quantity = number.clone().then(unit.or_not()).map(|(n, u)| {
            if let Literal::Number(num) = n {
                Literal::Quantity(num, u)
            } else {
                unreachable!()
            }
        });

        let literal = null
            .or(boolean)
            .or(string.clone())
            .or(quantity)
            .or(number)
            .or(datetime.or(date))
            .or(time)
            .map(Term::Literal);

        // Identifiers
        let identifier = text::ident().or(just('`')
            .ignore_then(none_of("`\\").or(just('\\').ignore_then(any())).repeated())
            .then_ignore(just('`'))
            .collect::<String>());

        // External constants
        let external_constant = just('%')
            .ignore_then(identifier.or(string.clone().map(|s| {
                if let Literal::String(str) = s {
                    str
                } else {
                    unreachable!()
                }
            })))
            .map(Term::ExternalConstant);

        // Function parameters
        let param_list = expr.clone().separated_by(just(',')).collect::<Vec<_>>();

        // Function invocation
        let function = identifier
            .clone()
            .then(
                just('(')
                    .ignore_then(param_list.or_not())
                    .then_ignore(just(')')),
            )
            .map(|(name, params)| Invocation::Function(name, params.unwrap_or_default()));

        // Invocations
        let invocation = choice((
            function,
            just("$this").to(Invocation::This),
            just("$index").to(Invocation::Index),
            just("$total").to(Invocation::Total),
            identifier.map(Invocation::Member),
        ))
        .map(Term::Invocation);

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
        let atom = term.map(Expression::Term);

        atom

        // Invocation expression (highest precedence)
        //       let invocation_expr = atom
        //               .clone()
        //               .then(
        //                   just('.')
        //                       .ignore_then(identifier.clone())
        //                       .repeated()
        //               )
        //               .map(|(expr, invocations)| {
        //                   invocations.into_iter().fold(expr, |acc, inv| {
        //                       Expression::Invocation(Box::new(acc), inv)
        //                   })
        //               });

        // Indexer expression
        //       let indexer_expr = invocation_expr
        //               .clone()
        //               .then(
        //                   expr.clone()
        //                       .delimited_by(just('['), just(']'))
        //                       .repeated()
        //               )
        //               .map(|(expr, indices)| {
        //                   indices.into_iter().fold(expr, |acc, idx| {
        //                       Expression::Indexer(Box::new(acc), Box::new(idx))
        //                   })
        //               });
        /*
                // Polarity expression
                let polarity_expr = choice((
                        just('+').or(just('-'))
                            .then(indexer_expr.clone())
                            .map(|(op, expr)| Expression::Polarity(op, Box::new(expr))),
                        indexer_expr,
                    ));

                // Multiplicative expression
                let op = choice((
                        just('*').to("*"),
                        just('/').to("/"),
                        text::keyword("div").to("div"),
                        text::keyword("mod").to("mod"),
                    ));

                let multiplicative_expr = polarity_expr
                        .clone()
                        .then(
                            op.then(polarity_expr.clone())
                                .repeated()
                        )
                        .map(|(first, rest)| {
                            rest.into_iter().fold(first, |lhs, (op, rhs)| {
                                Expression::Multiplicative(Box::new(lhs), op.to_string(), Box::new(rhs))
                            })
                        });

                // Additive expression
                let op = choice((
                        just('+').to("+"),
                        just('-').to("-"),
                        just('&').to("&"),
                    ));

                let additive_expr = multiplicative_expr
                        .clone()
                        .then(
                            op.then(multiplicative_expr.clone())
                                .repeated()
                        )
                        .map(|(first, rest)| {
                            rest.into_iter().fold(first, |lhs, (op, rhs)| {
                                Expression::Additive(Box::new(lhs), op.to_string(), Box::new(rhs))
                            })
                        });

                // Type expression
                let type_expr = additive_expr
                        .clone()
                        .then(
                            choice((
                                text::keyword("is"),
                                text::keyword("as"),
                            ))
                            .then(identifier.clone())
                            .or_not()
                        )
                        .map(|(expr, type_op)| {
                            if let Some((op, type_name)) = type_op {
                                Expression::Type(Box::new(expr), type_name)
                            } else {
                                expr
                            }
                        });

                // Union expression
                let union_expr = type_expr
                        .clone()
                        .then(
                            just('|')
                                .ignore_then(type_expr.clone())
                                .repeated()
                        )
                        .map(|(first, rest)| {
                            rest.into_iter().fold(first, |lhs, rhs| {
                                Expression::Union(Box::new(lhs), Box::new(rhs))
                            })
                        });

                // Inequality expression
                let op = choice((
                        just("<=").to("<="),
                        just("<").to("<"),
                        just(">=").to(">="),
                        just(">").to(">"),
                    ));

                let inequality_expr = union_expr
                        .clone()
                        .then(
                            op.then(union_expr.clone())
                                .or_not()
                        )
                        .map(|(lhs, rhs)| {
                            if let Some((op, rhs)) = rhs {
                                Expression::Inequality(Box::new(lhs), op.to_string(), Box::new(rhs))
                            } else {
                                lhs
                            }
                        });

                // Equality expression
                let op = choice((
                        just("=").to("="),
                        just("~").to("~"),
                        just("!=").to("!="),
                        just("!~").to("!~"),
                    ));

                let equality_expr = inequality_expr
                        .clone()
                        .then(
                            op.then(inequality_expr.clone())
                                .or_not()
                        )
                        .map(|(lhs, rhs)| {
                            if let Some((op, rhs)) = rhs {
                                Expression::Equality(Box::new(lhs), op.to_string(), Box::new(rhs))
                            } else {
                                lhs
                            }
                        });

                // Membership expression
                let op = choice((
                        text::keyword("in").to("in"),
                        text::keyword("contains").to("contains"),
                    ));

                let membership_expr = equality_expr
                        .clone()
                        .then(
                            op.then(equality_expr.clone())
                                .or_not()
                        )
                        .map(|(lhs, rhs)| {
                            if let Some((op, rhs)) = rhs {
                                Expression::Membership(Box::new(lhs), op.to_string(), Box::new(rhs))
                            } else {
                                lhs
                            }
                        });

                // And expression
                let and_expr = membership_expr
                        .clone()
                        .then(
                            text::keyword("and")
                                .ignore_then(membership_expr.clone())
                                .repeated()
                        )
                        .map(|(first, rest)| {
                            rest.into_iter().fold(first, |lhs, rhs| {
                                Expression::And(Box::new(lhs), Box::new(rhs))
                            })
                        });

                // Or expression
                let op = choice((
                        text::keyword("or").to("or"),
                        text::keyword("xor").to("xor"),
                    ));

                let or_expr = and_expr
                        .clone()
                        .then(
                            op.then(and_expr.clone())
                                .repeated()
                        )
                        .map(|(first, rest)| {
                            rest.into_iter().fold(first, |lhs, (op, rhs)| {
                                Expression::Or(Box::new(lhs), op.to_string(), Box::new(rhs))
                            })
                        });

                // Implies expression
                let implies_expr = or_expr
                        .clone()
                        .then(
                            text::keyword("implies")
                                .ignore_then(or_expr.clone())
                                .or_not()
                        )
                        .map(|(lhs, rhs)| {
                            if let Some(rhs) = rhs {
                                Expression::Implies(Box::new(lhs), Box::new(rhs))
                            } else {
                                lhs
                            }
                        });

                // Final expression
                implies_expr
        */
    })
    .padded()
}
