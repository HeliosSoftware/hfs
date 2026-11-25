//! One path or object-key component per tenant id, for the export and
//! bulk-submit output stores.
//!
//! The data stores derive their per-tenant namespaces injectively (see the S3
//! keyspace and the Elasticsearch index naming). The output stores joined the
//! raw id onto a root instead, so a hierarchical id such as `acme/research`
//! nested under `acme`, and purging `acme` swept its child's files too.

/// Maps a tenant id to exactly one path or key component, injectively.
///
/// An id made only of ASCII letters, digits, `-`, `_` and `.` (other than `.`
/// and `..` themselves) maps to itself, so every flat id keeps the directory
/// or prefix it has today. Any other id is percent-escaped byte by byte,
/// `.` included so that `.` and `..` cannot come back out as themselves,
/// and `%` and `/` as well, into one segment that no other id's segment can
/// equal or contain: an escaped segment always holds `%`, which a safe id
/// never does, and the escaping is injective on its own.
pub fn tenant_path_segment(tenant_id: &str) -> String {
    if is_safe(tenant_id) {
        return tenant_id.to_string();
    }
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(tenant_id.len() * 3);
    for &byte in tenant_id.as_bytes() {
        if is_safe_byte(byte) && byte != b'.' {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

fn is_safe_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

fn is_safe(tenant_id: &str) -> bool {
    !tenant_id.is_empty()
        && tenant_id != "."
        && tenant_id != ".."
        && tenant_id.bytes().all(is_safe_byte)
}

#[cfg(test)]
mod tests {
    use super::tenant_path_segment;

    #[test]
    fn flat_ids_keep_their_segment() {
        for id in ["default", "acme", "acme-1", "acme_1", "acme.v2", "T1"] {
            assert_eq!(tenant_path_segment(id), id);
        }
    }

    #[test]
    fn hierarchical_and_odd_ids_become_one_escaped_segment() {
        assert_eq!(tenant_path_segment("acme/research"), "acme%2Fresearch");
        assert_eq!(tenant_path_segment("a/exports"), "a%2Fexports");
        assert_eq!(tenant_path_segment("a%2Fb"), "a%252Fb");
        assert_eq!(tenant_path_segment("."), "%2E");
        assert_eq!(tenant_path_segment(".."), "%2E%2E");
        assert_eq!(tenant_path_segment("a b"), "a%20b");
        for id in ["acme/research", "a/", "/a", "a\\b", "ü"] {
            let segment = tenant_path_segment(id);
            assert!(
                !segment.contains('/') && !segment.contains('\\'),
                "{id:?} -> {segment:?}"
            );
        }
    }

    #[test]
    fn the_mapping_is_injective_on_colliding_ids() {
        let ids = [
            "a",
            "a/b",
            "a%2Fb",
            "a/exports",
            "a/",
            "/a",
            "a//b",
            "a.b",
            "a%2Eb",
            ".",
            "..",
            "a%",
            "a%25",
        ];
        for (i, left) in ids.iter().enumerate() {
            for right in ids.iter().skip(i + 1) {
                assert_ne!(
                    tenant_path_segment(left),
                    tenant_path_segment(right),
                    "{left:?} and {right:?} must not share a segment"
                );
            }
        }
    }

    #[test]
    fn no_segment_is_a_prefix_of_another_tenants_segment_plus_separator() {
        // The sweep prefix is `{segment}/`; `a` must not cover `a/exports`.
        let a = tenant_path_segment("a");
        let child = tenant_path_segment("a/exports");
        assert!(!format!("{child}/").starts_with(&format!("{a}/")));
    }
}
