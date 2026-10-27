use bitflags::bitflags;
use std::fmt;

bitflags! {
    /// Permission bits for SMART on FHIR v2 scopes.
    ///
    /// Each bit corresponds to one of the CRUDS operations:
    /// - `c` = Create
    /// - `r` = Read
    /// - `u` = Update
    /// - `d` = Delete
    /// - `s` = Search
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct SmartPermissions: u8 {
        /// Permission to create resources.
        const CREATE = 0b0000_0001;
        /// Permission to read resources by ID.
        const READ   = 0b0000_0010;
        /// Permission to update resources.
        const UPDATE = 0b0000_0100;
        /// Permission to delete resources.
        const DELETE = 0b0000_1000;
        /// Permission to search for resources.
        const SEARCH = 0b0001_0000;
    }
}

impl SmartPermissions {
    /// Parse a permission string from SMART v2 scope syntax.
    ///
    /// Each character maps to a permission: c, r, u, d, s.
    /// Returns `None` if any character is unrecognized.
    pub fn from_permission_str(s: &str) -> Option<Self> {
        let mut perms = SmartPermissions::empty();
        for ch in s.chars() {
            match ch {
                'c' => perms |= SmartPermissions::CREATE,
                'r' => perms |= SmartPermissions::READ,
                'u' => perms |= SmartPermissions::UPDATE,
                'd' => perms |= SmartPermissions::DELETE,
                's' => perms |= SmartPermissions::SEARCH,
                _ => return None,
            }
        }
        if perms.is_empty() { None } else { Some(perms) }
    }
}

impl fmt::Display for SmartPermissions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.contains(SmartPermissions::CREATE) {
            write!(f, "c")?;
        }
        if self.contains(SmartPermissions::READ) {
            write!(f, "r")?;
        }
        if self.contains(SmartPermissions::UPDATE) {
            write!(f, "u")?;
        }
        if self.contains(SmartPermissions::DELETE) {
            write!(f, "d")?;
        }
        if self.contains(SmartPermissions::SEARCH) {
            write!(f, "s")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_permissions() {
        assert_eq!(
            SmartPermissions::from_permission_str("r"),
            Some(SmartPermissions::READ)
        );
        assert_eq!(
            SmartPermissions::from_permission_str("c"),
            Some(SmartPermissions::CREATE)
        );
        assert_eq!(
            SmartPermissions::from_permission_str("s"),
            Some(SmartPermissions::SEARCH)
        );
    }

    #[test]
    fn test_parse_combined_permissions() {
        assert_eq!(
            SmartPermissions::from_permission_str("rs"),
            Some(SmartPermissions::READ | SmartPermissions::SEARCH)
        );
        assert_eq!(
            SmartPermissions::from_permission_str("cruds"),
            Some(
                SmartPermissions::CREATE
                    | SmartPermissions::READ
                    | SmartPermissions::UPDATE
                    | SmartPermissions::DELETE
                    | SmartPermissions::SEARCH
            )
        );
    }

    #[test]
    fn test_parse_invalid_permission() {
        assert_eq!(SmartPermissions::from_permission_str("x"), None);
        assert_eq!(SmartPermissions::from_permission_str("rz"), None);
        assert_eq!(SmartPermissions::from_permission_str(""), None);
    }

    #[test]
    fn test_display() {
        let perms = SmartPermissions::READ | SmartPermissions::SEARCH;
        assert_eq!(format!("{}", perms), "rs");

        let all = SmartPermissions::CREATE
            | SmartPermissions::READ
            | SmartPermissions::UPDATE
            | SmartPermissions::DELETE
            | SmartPermissions::SEARCH;
        assert_eq!(format!("{}", all), "cruds");
    }
}
