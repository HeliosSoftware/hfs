pub mod parser;
pub mod evaluator;
#[cfg(test)]
mod tests;

pub use parser::{Expression, Invocation, Literal, Term, parser};
pub use evaluator::{evaluate, EvaluationContext, EvaluationResult};
