mod parser;
#[cfg(test)]
mod tests;

pub use parser::{Expression, Literal, Term, Invocation, parser};

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests_original {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
