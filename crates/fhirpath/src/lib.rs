pub mod evaluator;
pub mod parser;

pub use evaluator::{EvaluationContext, EvaluationResult, evaluate};
pub use parser::{Expression, Invocation, Literal, Term, parser};
