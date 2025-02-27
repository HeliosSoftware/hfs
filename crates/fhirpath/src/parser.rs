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

        let boolean = text::keyword("true").to(Literal::Boolean(true))
            .or(text::keyword("false").to(Literal::Boolean(false)));

        let string = just('\'')
            .ignore_then(
                none_of("\'\\")
                    .or(just('\\').ignore_then(any()))
                    .repeated()
            )
            .then_ignore(just('\''))
            .collect::<String>()
            .map(Literal::String);

        // Number literals
        let number = text::int(10)
            .then(just('.').then(text::digits(10)).or_not())
            .map(|(i, d)| {
                if let Some((_, d)) = d {
                    Literal::Number(format!("{}.{}", i, d).parse().unwrap())
                } else {
                    Literal::Number(i.parse().unwrap())
                }
            });

        // Identifiers
        let identifier = text::ident()
            .or(
                just('`')
                    .ignore_then(
                        none_of("`\\")
                            .or(just('\\').ignore_then(any()))
                            .repeated()
                    )
                    .then_ignore(just('`'))
                    .collect::<String>()
            );

        // Member invocation
        let member = identifier.clone().map(Invocation::Member);
        
        // Basic term
        let term = choice((
            member.map(Term::Invocation),
            boolean.map(Term::Literal),
            string.map(Term::Literal),
            number.map(Term::Literal),
            null.map(Term::Literal),
        ));

        // Basic expression
        term.map(Expression::Term)
    })
    .padded()
}
