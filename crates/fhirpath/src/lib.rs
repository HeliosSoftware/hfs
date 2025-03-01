mod parser;
#[cfg(test)]
mod tests;

pub use parser::{parser, Expression, Invocation, Literal, Term};
