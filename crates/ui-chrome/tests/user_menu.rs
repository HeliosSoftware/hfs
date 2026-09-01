//! Behavioural tests for the shared account menu.
//!
//! These matter more than their size suggests. In both products every
//! `UserIdentity` field is hard-wired to the signed-out shape today —
//! `crates/ui/src/lib.rs` returns `None`/`false` from all five `user_*`
//! accessors while the browser login flow is still open (#320) — so the photo,
//! initials, display-name, secondary-line and sign-out branches of the template
//! are unreachable from either crate's own test suite. A stub `ChromeLabels`
//! plus a literal `UserIdentity` reaches all of them here, which is the only
//! place in the repository they are exercised at all.

use helios_ui_chrome::{ChromeLabels, UserIdentity, user_menu};

/// Locale-neutral labels: every key renders as `[key]`, so assertions read as
/// structure rather than as English, and the golden file does not churn when a
/// translation is reworded.
struct Stub {
    lang: &'static str,
}

impl Stub {
    fn new(lang: &'static str) -> Self {
        Self { lang }
    }
}

impl ChromeLabels for Stub {
    fn lang(&self) -> String {
        self.lang.to_string()
    }

    fn t(&self, key: &str) -> String {
        format!("[{key}]")
    }
}

/// A distinctive slice of `templates/icons/user.svg` — present iff the generic
/// avatar icon was rendered.
const USER_ICON: &str = "M8 2.75C6.75736 2.75";

/// The same, for `templates/icons/logout.svg`.
const LOGOUT_ICON: &str = "M3.25 2.75C3.25 2.19772";

fn render(lang: &'static str, user: UserIdentity<'_>) -> String {
    user_menu(&Stub::new(lang), user).expect("the shared partial must render")
}

/// The sentinel at the end of `partials/user-menu.html` is load-bearing: the
/// consuming layout supplies the newline after `</details>` (in HFS that is the
/// `\n      </div>` which followed the block at
/// `crates/ui/templates/layouts/base.html:268`). If the sentinel is ever
/// "tidied up", every HFS and HTS page grows a stray blank line and the
/// byte-for-byte identity with the pre-extraction output is lost.
#[test]
fn renders_without_a_trailing_newline() {
    let out = render("en", UserIdentity::default());

    assert!(
        out.ends_with("</details>"),
        "render must end at </details>, got: {:?}",
        &out[out.len().saturating_sub(60)..]
    );
    assert!(
        !out.ends_with('\n'),
        "render must not end with a newline; the whitespace-suppressing \
         sentinel at the end of templates/partials/user-menu.html is missing \
         or was reformatted"
    );
}

/// The fragment is spliced straight into a `.topbar__tools` container, so it
/// has to arrive already indented to the depth it had inside the HFS layout.
#[test]
fn starts_at_the_topbar_tools_indent() {
    let out = render("en", UserIdentity::default());

    assert!(
        out.starts_with("        <details class=\"menu menu--user\">"),
        "render must start with 8 spaces then the <details> element, got: {:?}",
        &out[..out.len().min(60)]
    );
}

/// Whole-output guard. A diff here is a diff in BOTH products at once, so read
/// it before accepting it.
///
/// To regenerate after a deliberate change:
/// `UPDATE_GOLDEN=1 cargo test -p helios-ui-chrome`. Rewriting the file by hand
/// defeats the point — it is meant to be a mechanical record of what the
/// template actually produces.
#[test]
fn structure_matches_the_golden() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/user-menu-structure.html"
    );
    let golden = include_str!("golden/user-menu-structure.html");
    let out = render("en", UserIdentity::default());

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        // `fs::write` performs no newline translation, so the LF the
        // `.gitattributes` rule requires survives on Windows too.
        std::fs::write(path, &out).expect("failed to rewrite the golden");
        return;
    }

    assert_eq!(
        out, golden,
        "the shared account menu no longer matches tests/golden/user-menu-structure.html; \
         this changes the rendered chrome of HFS *and* HTS. If the change is intended, \
         rerun with UPDATE_GOLDEN=1."
    );
}

/// Avatar fallback chain: photo wins, then initials, then the generic icon.
/// Each state is rendered twice — once in the `<summary>`, once in the account
/// card — and the counts below pin that down.
#[test]
fn avatar_falls_back_photo_then_initials_then_icon() {
    struct Case {
        name: &'static str,
        user: UserIdentity<'static>,
        photos: usize,
        initials: usize,
        icons: usize,
    }

    let cases = [
        Case {
            name: "photo wins over initials",
            user: UserIdentity {
                photo: Some("https://idp.example/avatar.png"),
                initials: Some("AB"),
                ..UserIdentity::default()
            },
            photos: 2,
            initials: 0,
            icons: 0,
        },
        Case {
            name: "initials when there is no photo",
            user: UserIdentity {
                initials: Some("AB"),
                ..UserIdentity::default()
            },
            photos: 0,
            initials: 2,
            icons: 0,
        },
        Case {
            name: "generic icon when there is neither",
            user: UserIdentity::default(),
            photos: 0,
            initials: 0,
            icons: 2,
        },
    ];

    for case in cases {
        let out = render("en", case.user);

        assert_eq!(
            out.matches(
                "<img class=\"topbar__avatar-img\" src=\"https://idp.example/avatar.png\" alt=\"\">"
            )
            .count(),
            case.photos,
            "{}: wrong number of avatar images",
            case.name
        );
        assert_eq!(
            out.matches("AB").count(),
            case.initials,
            "{}: wrong number of initials",
            case.name
        );
        assert_eq!(
            out.matches(USER_ICON).count(),
            case.icons,
            "{}: wrong number of generic user icons",
            case.name
        );
    }
}

/// The signed-in state nobody can reach from either product yet: `can_logout`
/// unlocks the sign-out link, and it points wherever the host says. HFS and HTS
/// mount their logout routes at different paths, which is precisely why the
/// href is a parameter and not the `/ui/logout` literal the extracted markup
/// used to hard-code.
#[test]
fn signed_in_renders_a_sign_out_link_at_the_given_href() {
    let out = render(
        "en",
        UserIdentity {
            display: Some("Ada Lovelace"),
            secondary: Some("ada@example.org"),
            can_logout: true,
            logout_href: "/hts/ui/logout",
            ..UserIdentity::default()
        },
    );

    assert!(
        out.contains("<a class=\"user-menu__out\" href=\"/hts/ui/logout\">"),
        "sign-out link missing or pointing at the wrong href:\n{out}"
    );
    assert!(
        out.contains(LOGOUT_ICON),
        "sign-out link should carry icons/logout.svg"
    );
    assert!(
        out.contains("<div class=\"user-menu__name\">Ada Lovelace</div>"),
        "display name should replace the anonymous label"
    );
    assert!(
        out.contains("<div class=\"user-menu__hint\">ada@example.org</div>"),
        "secondary line should replace the local-operator hint"
    );
}

/// Signed out: no link at all, regardless of what `logout_href` holds.
#[test]
fn signed_out_renders_no_sign_out_link() {
    let out = render(
        "en",
        UserIdentity {
            logout_href: "/ui/logout",
            ..UserIdentity::default()
        },
    );

    assert!(
        !out.contains("user-menu__out"),
        "sign-out link must not render when can_logout is false"
    );
    assert!(
        !out.contains("/ui/logout"),
        "logout_href must not leak into the output when can_logout is false"
    );
    assert!(
        out.contains("<div class=\"user-menu__name\">[user-anonymous]</div>"),
        "signed-out card should show the anonymous label"
    );
    assert!(
        out.contains("<div class=\"user-menu__hint\">[user-local-hint]</div>"),
        "signed-out card should show the local-operator hint"
    );
}

/// The language segmented control is a radio group in spirit: exactly one entry
/// is current, and it is the one matching `ChromeLabels::lang()`.
#[test]
fn exactly_one_language_is_marked_current() {
    for lang in ["en", "es", "de"] {
        let out = render(lang, UserIdentity::default());

        assert_eq!(
            out.matches("aria-current=\"true\"").count(),
            1,
            "lang={lang}: exactly one language entry must be current"
        );
        assert!(
            out.contains(&format!(
                "<a href=\"?lang={lang}\" aria-current=\"true\" aria-label=\"[language-{lang}]\">"
            )),
            "lang={lang}: the matching entry must be the current one:\n{out}"
        );

        for other in ["en", "es", "de"].iter().filter(|o| **o != lang) {
            assert!(
                out.contains(&format!(
                    "<a href=\"?lang={other}\" aria-label=\"[language-{other}]\">"
                )),
                "lang={lang}: entry {other} must render without aria-current"
            );
        }
    }
}

/// Standing XSS guard.
///
/// Consumers splice this fragment into their layout with `|safe`, which is only
/// sound while every value inside it has already been escaped by the template's
/// own HTML escaper. Nothing here is user-controlled *today*, but the whole
/// point of `UserIdentity` is that display names and secondary lines will soon
/// arrive as IdP claims (#724). If someone ever adds a `|safe` inside the
/// partial, or swaps the escaper off, this test is what notices.
#[test]
fn labels_are_html_escaped_before_the_safe_filter() {
    struct Hostile;

    impl ChromeLabels for Hostile {
        fn lang(&self) -> String {
            "en".to_string()
        }

        fn t(&self, _key: &str) -> String {
            "<script>x</script>".to_string()
        }
    }

    let out = user_menu(
        &Hostile,
        UserIdentity {
            display: Some("<script>alert(1)</script>"),
            secondary: Some("\"><img src=x onerror=alert(1)>"),
            ..UserIdentity::default()
        },
    )
    .expect("the shared partial must render");

    assert!(
        !out.contains("<script>"),
        "a raw <script> tag reached the output; the fragment is embedded with \
         |safe, so this is a live XSS:\n{out}"
    );
    assert!(
        out.contains("&#60;script&#62;x&#60;/script&#62;"),
        "label text should be escaped with numeric entities:\n{out}"
    );
    assert!(
        out.contains("&#60;script&#62;alert(1)&#60;/script&#62;"),
        "display name should be escaped:\n{out}"
    );
    assert!(
        !out.contains("onerror=alert(1)>"),
        "an attribute-breaking secondary line escaped its quoting:\n{out}"
    );
}
