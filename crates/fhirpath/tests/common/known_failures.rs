//! Declared exclusions for the in-process FHIRPath conformance suites.
//!
//! # Why this exists (issue #307)
//!
//! `r4_tests.rs` and `r5_tests.rs` end in `assert_eq!(failed_tests, 0)`. That
//! assertion is only meaningful if the set of tests excluded from the count is
//! *visible*. It used to be hardcoded in the test bodies — a `PrecisionDecimal`
//! name check, a `contains("conformsTo(")` substring match, a `contested_tests`
//! array that scored a wrong answer as a PASS. Those exclusions were real and
//! defensible, but nobody could see how many there were or whether the list was
//! still needed, so the suite could quietly stop checking things and stay green.
//!
//! Now every exclusion is an entry in a JSON file next to the corpus, and the
//! runner enforces three properties that a hardcoded list cannot:
//!
//! 1. an excluded test that **starts passing** fails the build ("delete this
//!    entry"), so the list cannot rot into pre-forgiveness for a re-regression;
//! 2. an entry matching **no test in the corpus** is reported, so an upstream
//!    rename cannot leave an entry silently protecting nothing;
//! 3. excluded tests are counted in their own bucket and are **never** added to
//!    the pass count.
//!
//! # This is NOT `known-test-failures.json`
//!
//! `data/r5/known-test-failures.json` is a *different* file with a different
//! owner: it is passed to the external .NET `fhirpath-validator.exe`
//! (`.github/workflows/ci.yml`, `--known-failures`) and describes that harness
//! running against `FHIR/fhir-test-cases@master`. This suite is in-process and
//! reads the *vendored* `data/{r4,r5}/tests-fhir-{r4,r5}.xml`. The two disagree
//! on purpose — `txTest03` is a declared failure for .NET (it checks output
//! *type* fidelity) while passing here, and `HTMLChecks` does not exist in the
//! vendored corpus at all. Sharing one file would make each consumer's gate lie
//! about the other's corpus, so the Rust suites own these files and the .NET
//! contract is left byte-identical.
//!
//! The schema is deliberately kept identical to that file, so an entry can be
//! moved between them by copy-paste when a limitation turns out to affect both.

#![allow(dead_code)] // Each `tests/*.rs` is its own binary; R4 and R5 use different subsets.

use std::collections::HashSet;

/// One declared exclusion.
///
/// Field names match `known-test-failures.json` exactly (`groupName`,
/// `testName`, `reason`) so entries are copy-pasteable between the two files.
#[derive(Debug, Clone)]
pub struct Exclusion {
    /// `<group name="...">` the test lives in.
    pub group_name: String,
    /// `<test name="...">`, or `*` for every test in the group.
    pub test_name: String,
    /// Why this test is not required to pass. Must say what would make it
    /// removable, not just that it fails.
    pub reason: String,
}

impl Exclusion {
    /// True when this entry covers a whole group rather than a single test.
    fn is_wildcard(&self) -> bool {
        self.test_name == "*"
    }

    fn matches(&self, group: &str, test: &str) -> bool {
        self.group_name == group && (self.is_wildcard() || self.test_name == test)
    }
}

/// The parsed exclusion set for one corpus, plus the bookkeeping needed to
/// detect stale entries.
#[derive(Debug)]
pub struct KnownFailures {
    entries: Vec<Exclusion>,
    /// Indices of entries that matched at least one test in the corpus.
    matched: HashSet<usize>,
    /// Indices of entries that matched at least one test that actually failed.
    justified: HashSet<usize>,
    label: String,
}

impl KnownFailures {
    /// Parses a `known-test-failures.json`-shaped document.
    ///
    /// Panics on a missing or malformed file rather than returning an empty set:
    /// an exclusion list that silently reads as "nothing is excluded" would make
    /// every excluded test fail at once, and one that fails open would suppress
    /// everything. Both are worse than a loud parse error. Callers pass the text
    /// via `include_str!`, so a missing file is a *compile* error and this only
    /// guards malformed content.
    pub fn parse(label: &str, json: &str) -> Self {
        let doc: serde_json::Value = serde_json::from_str(json).unwrap_or_else(|e| {
            panic!("{label}: declared-exclusions file is not valid JSON: {e}");
        });
        let list = doc
            .get("knownFailures")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| {
                panic!("{label}: declared-exclusions file has no `knownFailures` array");
            });

        let mut entries = Vec::with_capacity(list.len());
        for (i, raw) in list.iter().enumerate() {
            let field = |name: &str| -> String {
                raw.get(name)
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        panic!("{label}: knownFailures[{i}] is missing string field `{name}`");
                    })
                    .trim()
                    .to_string()
            };
            let reason = field("reason");
            assert!(
                reason.len() >= 12,
                "{label}: knownFailures[{i}] has a reason too short to be useful: {reason:?}. \
                 Say what the limitation is and what would make this entry removable."
            );
            entries.push(Exclusion {
                group_name: field("groupName"),
                test_name: field("testName"),
                reason,
            });
        }

        Self {
            entries,
            matched: HashSet::new(),
            justified: HashSet::new(),
            label: label.to_string(),
        }
    }

    /// Number of declared entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is declared.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Looks up an exclusion for `group::test`, recording that the entry matched
    /// a real test so [`Self::unmatched`] can report the ones that did not.
    ///
    /// `failed` says whether the test would have failed without the exclusion.
    /// Only a failing test *justifies* an entry; see [`Self::unjustified`].
    pub fn lookup(&mut self, group: &str, test: &str, failed: bool) -> Option<String> {
        // Exact entries win over wildcards so a group-wide exclusion does not
        // mask a more specific reason.
        let idx = self
            .entries
            .iter()
            .position(|e| e.matches(group, test) && !e.is_wildcard())
            .or_else(|| self.entries.iter().position(|e| e.matches(group, test)))?;

        self.matched.insert(idx);
        if failed {
            self.justified.insert(idx);
        }
        Some(self.entries[idx].reason.clone())
    }

    /// Entries that matched no test in the corpus at all.
    ///
    /// Usually an upstream rename or a typo: the entry protects nothing, and the
    /// failure it was written for will reappear later looking brand new.
    pub fn unmatched(&self) -> Vec<String> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.matched.contains(i))
            .map(|(_, e)| format!("{}::{} — {}", e.group_name, e.test_name, e.reason))
            .collect()
    }

    /// Exact (non-wildcard) entries whose test ran and **passed**.
    ///
    /// The entry is obsolete and must be deleted, otherwise it silently
    /// pre-forgives the next regression of a test that currently works.
    ///
    /// Wildcard entries are exempt: a group-level exclusion legitimately spans a
    /// mix of passing and failing tests. `testConformsTo` is the live example —
    /// `testConformsTo3` is `invalid="execution"`, so an unimplemented
    /// `conformsTo()` makes it pass (for the wrong reason) while its two
    /// siblings fail.
    pub fn unjustified(&self) -> Vec<String> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(i, e)| {
                !e.is_wildcard() && self.matched.contains(i) && !self.justified.contains(i)
            })
            .map(|(_, e)| format!("{}::{} — {}", e.group_name, e.test_name, e.reason))
            .collect()
    }

    /// A short label for diagnostics (e.g. `"R5"`).
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// How a single conformance test was scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Ran and matched its declared expectation.
    Pass,
    /// Ran and did not match. Fails the suite.
    Fail,
    /// Ran, did not match, and is declared in the exclusions file.
    KnownFail,
    /// Structurally not a test (empty `<expression>`). Never a defect-shaped skip.
    Skipped,
}

/// Running totals for one suite.
#[derive(Debug, Default)]
pub struct Tally {
    pub passed: usize,
    pub failed: usize,
    pub known_fail: usize,
    pub skipped: usize,
    /// `group::test — detail` for each undeclared failure.
    pub failures: Vec<String>,
    /// `group::test — reason` for each declared failure that fired.
    pub excluded: Vec<String>,
}

impl Tally {
    /// Every test that reached a verdict, including skips.
    pub fn total(&self) -> usize {
        self.passed + self.failed + self.known_fail + self.skipped
    }

    pub fn record(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Pass => self.passed += 1,
            Outcome::Fail => self.failed += 1,
            Outcome::KnownFail => self.known_fail += 1,
            Outcome::Skipped => self.skipped += 1,
        }
    }

    /// Emits the machine-readable one-liner CI greps for, then the human summary.
    pub fn report(&self, label: &str) {
        // Single line, stable shape: `CONFORMANCE R5: total=1035 pass=... `.
        // CI asserts on this rather than on a `#[test]` count, because the unit
        // that can silently vanish here is a *corpus entry*, not a Rust test.
        println!(
            "CONFORMANCE {label}: total={} pass={} fail={} known_fail={} skipped={}",
            self.total(),
            self.passed,
            self.failed,
            self.known_fail,
            self.skipped
        );
        println!("\n{label} conformance summary:");
        println!("  Total declared tests : {}", self.total());
        println!("  Passed               : {}", self.passed);
        println!("  Failed (undeclared)  : {}", self.failed);
        println!("  Known failures       : {}", self.known_fail);
        println!("  Skipped (structural) : {}", self.skipped);
        if !self.excluded.is_empty() {
            println!("\n  Declared exclusions that fired:");
            for e in &self.excluded {
                println!("    - {e}");
            }
        }
    }

    /// The suite's final verdict.
    ///
    /// Deliberately checks more than "no failures": a corpus that shrank to
    /// nothing, or an exclusion list that stopped matching, would otherwise
    /// produce a green run that verified nothing — the exact failure mode
    /// issue #307 was filed about.
    pub fn assert_conformant(&self, kf: &KnownFailures, min_tests: usize, min_passing: usize) {
        // 1. The corpus actually loaded. A vacuous suite must not pass.
        assert!(
            self.total() >= min_tests,
            "{}: only {} tests were executed but the corpus declares at least {min_tests}. \
             The test corpus failed to load or has shrunk; refusing to report a green run \
             that checked almost nothing.",
            kf.label(),
            self.total(),
        );
        assert!(
            self.passed >= min_passing,
            "{}: only {} tests passed (floor {min_passing}). Even if nothing is listed as \
             failed, this many passes means the suite is not really running.",
            kf.label(),
            self.passed,
        );

        // 2. Exclusion entries that match nothing protect nothing.
        let unmatched = kf.unmatched();
        assert!(
            unmatched.is_empty(),
            "{}: {} declared exclusion(s) match no test in the vendored corpus. \
             An entry that matches nothing silently stops excluding anything, and the failure \
             it was written for will reappear later looking brand new. Fix the group/test name \
             or delete the entry:\n  {}",
            kf.label(),
            unmatched.len(),
            unmatched.join("\n  "),
        );

        // 3. Exclusions for tests that now pass are obsolete.
        let unjustified = kf.unjustified();
        assert!(
            unjustified.is_empty(),
            "{}: {} declared exclusion(s) name a test that now PASSES. Delete these entries — \
             leaving them in place silently pre-forgives the next regression of a test that \
             currently works:\n  {}",
            kf.label(),
            unjustified.len(),
            unjustified.join("\n  "),
        );

        // 4. The actual conformance assertion.
        assert_eq!(
            self.failed,
            0,
            "{}: {} test(s) failed and are not declared in the exclusions file. \
             Fix the defect, or add an entry with a reason saying what would make it removable. \
             Do not delete the test to get green:\n  {}",
            kf.label(),
            self.failed,
            self.failures.join("\n  "),
        );
    }
}
