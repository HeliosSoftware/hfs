/// Type information result for FHIRPath type() function
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeInfoResult {
    pub namespace: String,
    pub name: String,
}

impl TypeInfoResult {
    pub fn new(namespace: &str, name: &str) -> Self {
        Self {
            namespace: namespace.to_string(),
            name: name.to_string(),
        }
    }
}
