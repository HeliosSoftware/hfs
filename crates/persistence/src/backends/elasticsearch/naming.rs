//! The single tenant → Elasticsearch index-name derivation.
//!
//! # Why this module exists
//!
//! Every Elasticsearch operation addresses documents through an index name (and,
//! for the `_id`-addressed operations, a document id *within* that index). If two
//! distinct tenants can ever produce the same index name, then `create`,
//! `update`, `delete`, and the `create_or_update` existence probe — none of which
//! can carry a query filter, because `GET /{index}/_doc/{id}` admits none — will
//! read, overwrite, and delete each other's documents.
//!
//! The previous derivation was `format!("{prefix}_{tenant.to_lowercase()}_{type}")`.
//! `to_lowercase()` is not injective, so tenants `ACME` and `acme` shared an
//! index (issue #384). It was also *partial*: a tenant id containing `/` produced
//! a string Elasticsearch rejects outright, so every write for such a tenant
//! failed with a 500.
//!
//! [`encode_tenant_segment`] replaces it with a total, injective encoding.
//!
//! # The encoding
//!
//! Bytes of the UTF-8 encoding of the tenant id are mapped as follows:
//!
//! - a byte in the **safe set** `{a–z, 0–9, '-', '_', '.'}` is emitted verbatim;
//! - every other byte is emitted as `+` followed by two **lowercase** hex digits.
//!
//! `+` is not in the safe set, so a literal `+` is itself escaped (`+2b`) and can
//! never be confused with an escape introducer.
//!
//! ## Injectivity
//!
//! [`decode_tenant_segment`] is a total left inverse: scan left to right; on `+`,
//! consume exactly three bytes and emit the byte the two hex digits denote;
//! otherwise emit the byte. This is unambiguous because `+` never appears
//! un-escaped and escapes are fixed-width. A left inverse implies injectivity, so
//! distinct tenant ids always produce distinct segments. The escape is therefore
//! not forgeable either: `encode("+2f") == "+2b2f" != "+2f" == encode("/")`.
//!
//! This mirrors the pattern issue #271 established for the S3 tenant registry key
//! (`backends::s3::keyspace::registry_object_id`): an injective escape whose
//! introducer is escaped first, identity everywhere else.
//!
//! ## Legality as an Elasticsearch index name
//!
//! Elasticsearch requires index names to be lowercase; to exclude
//! `\ / * ? " < > | `, space, `,` and `#`; to not begin with `-`, `_` or `+`; to
//! not be `.` or `..`; and to be at most 255 bytes.
//!
//! The output alphabet is `{a–z, 0–9, '-', '_', '.', '+'}`, which contains none
//! of the excluded characters and no uppercase byte. The leading-character and
//! `.`/`..` rules are discharged by the *index prefix*, which always precedes the
//! tenant segment and is validated by [`validate_index_prefix`]. See
//! [`encode_tenant_segment`] for the length caveat.
//!
//! ## Why `+` and not `%`
//!
//! Both are legal in an Elasticsearch index name and both survive the client's
//! path encoder. `+` is preferred because (a) no HFS tenant-routing surface
//! accepts it, so it is only ever produced here and never carried in by a user,
//! and (b) `%` in a URL path is the classic double-decode hazard — an
//! intermediary that decodes once and re-forwards would turn `a%252Fb` into
//! `a%2Fb`, which Elasticsearch would then decode to a name containing `/`. A
//! spurious extra decode of `%2B` merely yields `+`, which is inert.
//!
//! # Identity on already-safe ids — a load-bearing property
//!
//! For any tenant id drawn from `[a-z0-9._-]*`, the encoding is the **identity**,
//! so the index name is byte-identical to the one the old derivation produced.
//! Deployments whose tenant ids are already lowercase therefore see no index
//! rename, no reindex, and no change of any kind on upgrade.
//!
//! The ids whose index names *do* change are exactly those that are already
//! broken today: mixed-case ids (colliding), ids containing `/` (500 on every
//! write), and exotic ids reachable only through the unvalidated JWT tenant claim
//! (see issue #385). The blast radius of the fix equals the blast radius of the
//! bug. This is asserted by
//! `already_safe_tenant_ids_are_unchanged_so_conforming_deployments_do_not_migrate`.

/// Characters emitted verbatim by [`encode_tenant_segment`].
///
/// Chosen as the intersection of "legal in an Elasticsearch index name",
/// "lowercase", and "commonly present in a tenant id" — so that the encoding is
/// the identity on ids that are already safe.
///
/// `_` is deliberately included even though it is also the field separator in
/// `{prefix}_{tenant}_{type}`. Excluding it would rename the index of every
/// deployment using a `my_tenant`-shaped id — the most common legitimate shape,
/// which has no bug — in exchange for making the tenant glob exact. That trade is
/// rejected: the glob's over-match (tenant `a`'s pattern `hfs_a_*` also matches
/// tenant `a_b`'s indices) is not a leak, because every glob-scoped query carries
/// a `{"term": {"tenant_id": …}}` filter on a `keyword` field, and that filter —
/// not the glob — is what isolates tenants.
fn is_safe_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
}

/// The escape introducer. Not a member of the safe set, so it is self-escaping.
const ESCAPE: u8 = b'+';

/// Encodes a tenant id into a single, injective, Elasticsearch-legal index-name
/// segment.
///
/// See the module documentation for the encoding, its injectivity proof, and why
/// it is the identity on already-safe ids.
///
/// # Length
///
/// The encoding expands by at most 3× per byte. The full index name must be
/// ≤255 bytes, which holds comfortably for every id the validated routing
/// surfaces accept (`crates/rest/src/middleware/tenant_prefix.rs` caps at 64
/// characters; `crates/rest/src/handlers/admin_tenants.rs` at 128). An id long
/// enough to overflow can only arrive through the unvalidated JWT tenant claim,
/// and Elasticsearch rejects the oversized name with a 400 that surfaces as a
/// storage error — loud and non-lossy. Truncating here to fit would reintroduce
/// exactly the non-injective derivation this module exists to remove, so it is
/// deliberately not done. Bounding the id globally is issue #385's job.
pub(crate) fn encode_tenant_segment(tenant_id: &str) -> String {
    let bytes = tenant_id.as_bytes();
    // Fast path: an already-safe id is returned unchanged without allocating a
    // byte at a time. This is the overwhelmingly common case on every request.
    if bytes.iter().all(|b| is_safe_byte(*b)) {
        return tenant_id.to_string();
    }

    // Lowercase hex: the whole index name must be lowercase, so the digits are
    // written from this table rather than via `{:X}`.
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut out = String::with_capacity(bytes.len() + 8);
    for &b in bytes {
        if is_safe_byte(b) {
            out.push(b as char);
        } else {
            out.push(ESCAPE as char);
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    out
}

/// The left inverse of [`encode_tenant_segment`].
///
/// Exists to make injectivity a *tested* property rather than an argued one —
/// `decode(encode(x)) == x` over an adversarial corpus is what the unit tests
/// assert. Returns `None` for input that is not well-formed encoder output.
#[cfg(test)]
pub(crate) fn decode_tenant_segment(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == ESCAPE {
            // Escapes are fixed-width, so there is no alignment ambiguity.
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Builds the index name for one tenant and resource type.
///
/// `resource_type` is lowercased rather than escaped. That is safe — and, unlike
/// the tenant id, *forced*: Elasticsearch index names must be lowercase, and FHIR
/// resource types are a closed, fixed set whose members never differ only by
/// case, so lowercasing is injective over the actual domain.
pub(crate) fn index_name(index_prefix: &str, tenant_id: &str, resource_type: &str) -> String {
    format!(
        "{}_{}_{}",
        index_prefix,
        encode_tenant_segment(tenant_id),
        resource_type.to_lowercase()
    )
}

/// Builds the glob matching every index belonging to one tenant.
///
/// **This must stay derived from the same encoder as [`index_name`].** If the two
/// ever diverge, the glob stops matching the indices that exist and the
/// glob-scoped operations fail silently in the worst possible direction:
/// `purge_tenant_data` deletes nothing and reports success, and `count` returns
/// zero. That is why all four formerly hand-rolled `{prefix}_{tenant}_*` literals
/// now route through this function.
///
/// The glob narrows *which indices are scanned*; the `{"term": {"tenant_id": …}}`
/// filter that every caller supplies is what *isolates the tenant*. See
/// [`is_safe_byte`] for why the glob is deliberately allowed to over-match.
pub(crate) fn tenant_index_pattern(index_prefix: &str, tenant_id: &str) -> String {
    format!("{}_{}_*", index_prefix, encode_tenant_segment(tenant_id))
}

/// Validates an operator-supplied index prefix.
///
/// The legality proofs in this module lean on the prefix: it is what guarantees
/// an index name never begins with `-`, `_` or `+` and is never `.` or `..`,
/// since the prefix always comes first. Validating it once here discharges both
/// obligations for every name the module produces.
pub(crate) fn validate_index_prefix(prefix: &str) -> Result<(), String> {
    let first = prefix.bytes().next().ok_or_else(|| {
        "Elasticsearch index prefix must not be empty (it is what keeps every index name from \
         starting with a reserved character)"
            .to_string()
    })?;
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(format!(
            "Elasticsearch index prefix {prefix:?} must start with a lowercase letter or digit; \
             Elasticsearch rejects index names beginning with '-', '_' or '+'"
        ));
    }
    if let Some(bad) = prefix.bytes().find(|b| !is_safe_byte(*b)) {
        return Err(format!(
            "Elasticsearch index prefix {prefix:?} contains the illegal byte {:?}; only \
             lowercase letters, digits, '-', '_' and '.' are allowed",
            bad as char
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tenant ids chosen so that each one kills a specific way a derivation can
    /// lose information. A pair that a broken derivation would conflate must be
    /// present for every such mechanism, or the injectivity test passes
    /// vacuously.
    const COLLIDING_TENANTS: &[&str] = &[
        // ASCII case — the actual #384 defect.
        "acme",
        "ACME",
        "AcMe",
        // Separator ambiguity in `{prefix}_{tenant}_{type}`.
        "a",
        "a_b",
        "a_b_c",
        // The classes a lossy sanitiser (`'/' | '\\' | ' ' => '_'`) collapses.
        "a/b",
        "a\\b",
        "a b",
        // Escape forgery: a literal spelling of an escape must not collide with
        // the thing it would encode.
        "a+2fb",
        "+2f",
        "/",
        // Leading/trailing separators — kills any `trim_matches('/')`.
        "/a",
        "a/",
        "//a",
        // Unicode normalisation — kills a "fix" that NFKC-normalises rather than
        // escapes. U+FB01 vs "fi", and U+00C5 vs U+212B.
        "\u{fb01}le",
        "file",
        "\u{c5}",
        "\u{212b}",
        // Non-ASCII generally.
        "café",
        // Truncation — two ids sharing a long common prefix.
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-x",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-y",
        // Control-plane and hierarchy shapes that reach storage unvalidated.
        "__system__",
        "acme/research",
        "tenant-123",
        "my_tenant",
    ];

    const TYPES: &[&str] = &["Patient", "Observation", "DiagnosticReport"];

    /// Every character Elasticsearch forbids in an index name.
    const ES_FORBIDDEN: &[char] = &['\\', '/', '*', '?', '"', '<', '>', '|', ' ', ',', '#'];

    /// The core invariant. Distinct tenants must never address the same index —
    /// this is what makes every `_id`-addressed operation tenant-confined, and it
    /// is why `document_id` needs no tenant component.
    #[test]
    fn index_name_is_injective_in_the_tenant() {
        for rt in TYPES {
            for (i, a) in COLLIDING_TENANTS.iter().enumerate() {
                for b in COLLIDING_TENANTS.iter().skip(i + 1) {
                    assert_ne!(
                        index_name("hfs", a, rt),
                        index_name("hfs", b, rt),
                        "tenants {a:?} and {b:?} must not share an index for {rt}"
                    );
                }
            }
        }
    }

    /// Injectivity stated as its mechanism, so a future encoding change has to
    /// keep the property rather than just keep the test green: a total left
    /// inverse exists.
    #[test]
    fn encoding_round_trips_so_it_cannot_be_lossy() {
        for t in COLLIDING_TENANTS {
            let encoded = encode_tenant_segment(t);
            assert_eq!(
                decode_tenant_segment(&encoded).as_deref(),
                Some(*t),
                "encode/decode must round-trip for {t:?} (encoded as {encoded:?})"
            );
        }
    }

    /// The escape must not be forgeable: a tenant that literally spells an escape
    /// sequence must not collide with the tenant that encoding produces it from.
    #[test]
    fn escape_sequences_cannot_be_forged() {
        assert_ne!(encode_tenant_segment("/"), encode_tenant_segment("+2f"));
        assert_ne!(encode_tenant_segment("a/b"), encode_tenant_segment("a+2fb"));
        // The introducer is itself escaped, which is what makes the above hold.
        assert_eq!(encode_tenant_segment("+"), "+2b");
    }

    /// Producing an injective name is not enough — Elasticsearch has to accept
    /// it. This models the cluster's rules; the integration test is the oracle.
    #[test]
    fn every_index_name_is_legal_for_elasticsearch() {
        for rt in TYPES {
            for t in COLLIDING_TENANTS {
                let name = index_name("hfs", t, rt);
                assert_eq!(name, name.to_lowercase(), "{name:?} must be lowercase");
                for c in ES_FORBIDDEN {
                    assert!(
                        !name.contains(*c),
                        "{name:?} must not contain {c:?} (tenant {t:?})"
                    );
                }
                assert!(
                    !name.starts_with('-') && !name.starts_with('_') && !name.starts_with('+'),
                    "{name:?} must not start with a reserved character"
                );
                assert!(name != "." && name != "..", "{name:?} is a reserved name");
                assert!(
                    name.len() <= 255,
                    "{name:?} is {} bytes; Elasticsearch caps index names at 255",
                    name.len()
                );
            }
        }
    }

    /// The property that confines the upgrade's blast radius to deployments that
    /// are already broken. If this fails, every conforming deployment on earth
    /// needs a reindex — see the module docs.
    #[test]
    fn already_safe_tenant_ids_are_unchanged_so_conforming_deployments_do_not_migrate() {
        for t in ["acme", "default", "tenant-123", "my_tenant", "t.1", "a1"] {
            assert_eq!(encode_tenant_segment(t), t, "{t:?} must encode to itself");
            // Byte-identical to what the pre-fix `to_lowercase()` derivation
            // produced for these ids.
            assert_eq!(
                index_name("hfs", t, "Patient"),
                format!("hfs_{}_patient", t.to_lowercase())
            );
        }
    }

    /// One golden value, deliberately, so the encoding's readability is
    /// reviewable. Safe because `acme` is already lowercase and slash-free, so
    /// this constrains nothing about case or escape handling and cannot re-pin
    /// the #384 defect the way the test it replaces did.
    #[test]
    fn plain_lowercase_tenant_index_name_documents_the_shape() {
        assert_eq!(index_name("hfs", "acme", "Patient"), "hfs_acme_patient");
    }

    /// Regression for #384 as filed: the two ids named in the issue must land in
    /// different indices. The test this replaces asserted the opposite.
    #[test]
    fn case_variant_tenants_do_not_share_an_index() {
        assert_ne!(
            index_name("hfs", "ACME", "Observation"),
            index_name("hfs", "acme", "Observation")
        );
    }

    /// The other half of #384: a hierarchical id used to produce a string
    /// Elasticsearch rejects, so every write 500'd.
    #[test]
    fn hierarchical_tenant_ids_produce_a_legal_index_name() {
        let name = index_name("hfs", "acme/research", "Patient");
        assert_eq!(name, "hfs_acme+2fresearch_patient");
        assert!(!name.contains('/'));
    }

    /// The glob and the exact name must agree, or glob-scoped operations sweep
    /// indices that do not exist — `purge_tenant_data` silently purging nothing.
    #[test]
    fn tenant_pattern_matches_every_index_name_for_that_tenant() {
        for t in COLLIDING_TENANTS {
            let pattern = tenant_index_pattern("hfs", t);
            let stem = pattern
                .strip_suffix('*')
                .expect("pattern is a prefix glob by construction");
            for rt in TYPES {
                let name = index_name("hfs", t, rt);
                assert!(
                    name.starts_with(stem),
                    "pattern {pattern:?} must match index {name:?}"
                );
            }
        }
    }

    /// The converse hazard: a pattern that matches a case-variant tenant's
    /// indices would re-open #384 through the glob paths.
    #[test]
    fn tenant_pattern_does_not_match_a_case_variant_tenants_index() {
        let stem = tenant_index_pattern("hfs", "acme");
        let stem = stem.strip_suffix('*').unwrap();
        assert!(!index_name("hfs", "ACME", "Patient").starts_with(stem));
    }

    /// The index template registers mappings against `{prefix}_*`. A name outside
    /// that glob would be auto-created with *dynamic* mapping instead — no error,
    /// no exception, search quality silently degraded.
    #[test]
    fn every_index_name_is_covered_by_the_index_template_glob() {
        for t in COLLIDING_TENANTS {
            for rt in TYPES {
                assert!(
                    index_name("hfs", t, rt).starts_with("hfs_"),
                    "index for {t:?}/{rt} must fall under the template pattern"
                );
            }
        }
    }

    /// The startup diagnostic in `schema.rs` decides "is this document in the
    /// right index?" by re-encoding its `tenant_id` and comparing against the
    /// index's own tenant segment. That only detects the #384 collision because
    /// the encoder maps `ACME` somewhere other than `acme` — which the old
    /// lowercasing derivation did not.
    #[test]
    fn re_encoding_a_tenant_id_identifies_a_misplaced_document() {
        // A document written by tenant `ACME` under the pre-fix derivation sits
        // in the index whose tenant segment is `acme`. Re-encoding must disagree.
        assert_ne!(encode_tenant_segment("ACME"), "acme");
        // A document written by tenant `acme` sits where it belongs.
        assert_eq!(encode_tenant_segment("acme"), "acme");
    }

    #[test]
    fn index_prefix_validation_rejects_prefixes_that_break_the_legality_proof() {
        assert!(validate_index_prefix("hfs").is_ok());
        assert!(validate_index_prefix("hfs-prod").is_ok());
        assert!(validate_index_prefix("h2").is_ok());
        // Empty would let the tenant segment lead the index name.
        assert!(validate_index_prefix("").is_err());
        // Elasticsearch rejects names beginning with these.
        assert!(validate_index_prefix("_hfs").is_err());
        assert!(validate_index_prefix("-hfs").is_err());
        assert!(validate_index_prefix("+hfs").is_err());
        // Uppercase and forbidden characters.
        assert!(validate_index_prefix("HFS").is_err());
        assert!(validate_index_prefix("hfs/prod").is_err());
        assert!(validate_index_prefix("hfs prod").is_err());
    }
}
