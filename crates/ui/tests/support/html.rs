//! Parsed-HTML assertions for `helios-ui`'s HTTP tests.
//!
//! A thin layer over [`scraper`], so a test asks for elements by CSS selector
//! and reads their attributes and text as the browser would — attribute
//! entities decoded, attribute order and whitespace irrelevant — instead of
//! searching the markup as a string.
//!
//! Lives under `tests/support/` with the other shared test code, but is
//! included on its own (`#[path = "support/html.rs"] mod html;`) so a binary
//! that only needs parsing does not also compile the settings-store double.
//!
//! - [`Dom::page`] / [`Dom::fragment`] parse a full response or an htmx
//!   fragment;
//! - [`Dom::one`] selects exactly one element (panicking with the selector and
//!   a snippet when none or several match), [`Dom::all`] every match,
//!   [`Dom::count`] how many;
//! - [`Dom::root`] is a fragment's single top-level element;
//! - [`Dom::text`] is the text of the whole tree;
//! - on an element, [`El::attr`], [`El::has_attr`] (boolean attributes such
//!   as `open` or `hx-preserve`), [`El::has_class`], [`El::text`]
//!   (whitespace-collapsed), [`El::is`] (matches a selector itself), and
//!   scoped [`El::one`] / [`El::all`].

// Each test binary includes this module and uses a different subset of it.
#![allow(dead_code)]

use scraper::{ElementRef, Html, Node, Selector};

/// A parsed page or fragment.
pub struct Dom {
    html: Html,
    source_len: usize,
}

/// One element of a [`Dom`].
#[derive(Clone, Copy)]
pub struct El<'a>(ElementRef<'a>);

fn selector(css: &str) -> Selector {
    Selector::parse(css).unwrap_or_else(|e| panic!("invalid selector {css:?}: {e:?}"))
}

/// At most `limit` characters of `markup`, for failure messages.
fn snippet(markup: &str, limit: usize) -> String {
    if markup.chars().count() <= limit {
        markup.to_string()
    } else {
        let head: String = markup.chars().take(limit).collect();
        format!("{head}…")
    }
}

fn exactly_one<'a>(css: &str, found: Vec<ElementRef<'a>>, context: impl Fn() -> String) -> El<'a> {
    match found.as_slice() {
        [one] => El(*one),
        [] => panic!("no element matches {css:?} in: {}", context()),
        many => panic!(
            "{} elements match {css:?}, expected one; the first two: {} | {}",
            many.len(),
            snippet(&many[0].html(), 300),
            snippet(&many[1].html(), 300)
        ),
    }
}

/// Whitespace runs collapsed to one space, trimmed.
fn collapse(text: impl Iterator<Item = impl AsRef<str>>) -> String {
    let joined: String = text.map(|t| t.as_ref().to_string()).collect();
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Dom {
    /// Parses a whole response body as a document.
    pub fn page(source: &str) -> Self {
        Self {
            html: Html::parse_document(source),
            source_len: source.len(),
        }
    }

    /// Parses an htmx fragment response (in a `<body>` context).
    pub fn fragment(source: &str) -> Self {
        Self {
            html: Html::parse_fragment(source),
            source_len: source.len(),
        }
    }

    /// Every element matching `css`, in document order.
    pub fn all(&self, css: &str) -> Vec<El<'_>> {
        self.html.select(&selector(css)).map(El).collect()
    }

    /// How many elements match `css`.
    pub fn count(&self, css: &str) -> usize {
        self.html.select(&selector(css)).count()
    }

    /// The one element matching `css`; panics when none or several do.
    pub fn one(&self, css: &str) -> El<'_> {
        exactly_one(css, self.html.select(&selector(css)).collect(), || {
            format!(
                "a {}-byte body starting {}",
                self.source_len,
                snippet(&self.html.html(), 400)
            )
        })
    }

    /// The text of the whole tree, whitespace-collapsed.
    pub fn text(&self) -> String {
        collapse(self.html.root_element().text())
    }

    /// A fragment's single top-level element. Panics when the fragment has
    /// several top-level elements, none, or top-level text beside it — so a
    /// match also proves nothing (no layout, no trailing markup) surrounds it.
    pub fn root(&self) -> El<'_> {
        // A fragment parses under a synthetic `<html>` element.
        let container = self.html.root_element();
        let mut elements = Vec::new();
        for child in container.children() {
            match child.value() {
                Node::Element(_) => elements.push(ElementRef::wrap(child).unwrap()),
                Node::Text(text) if text.trim().is_empty() => {}
                Node::Comment(_) => {}
                other => panic!("top-level content beside the fragment root: {other:?}"),
            }
        }
        exactly_one(":root > *", elements, || {
            format!(
                "a fragment starting {}",
                snippet(&container.inner_html(), 400)
            )
        })
    }
}

impl<'a> El<'a> {
    /// The value of attribute `name`, entities decoded.
    pub fn attr(&self, name: &str) -> Option<&'a str> {
        self.0.value().attr(name)
    }

    /// Whether the element carries attribute `name` at all (a boolean
    /// attribute such as `open` or `hx-preserve`).
    pub fn has_attr(&self, name: &str) -> bool {
        self.0.value().attr(name).is_some()
    }

    /// Whether the element's `class` list contains `class`.
    pub fn has_class(&self, class: &str) -> bool {
        self.0.value().classes().any(|c| c == class)
    }

    /// The names of every attribute the element carries.
    pub fn attr_names(&self) -> Vec<&'a str> {
        self.0.value().attrs().map(|(name, _)| name).collect()
    }

    /// The element's tag name, lowercase.
    pub fn name(&self) -> &'a str {
        self.0.value().name()
    }

    /// The element's text, whitespace-collapsed and trimmed.
    pub fn text(&self) -> String {
        collapse(self.0.text())
    }

    /// The element's own text — its direct text children only, not its
    /// descendants' — whitespace-collapsed and trimmed.
    pub fn own_text(&self) -> String {
        collapse(
            self.0
                .children()
                .filter_map(|child| child.value().as_text().map(|t| t.to_string())),
        )
    }

    /// Every descendant matching `css`.
    pub fn all(&self, css: &str) -> Vec<El<'a>> {
        self.0.select(&selector(css)).map(El).collect()
    }

    /// The one descendant matching `css`; panics when none or several do.
    pub fn one(&self, css: &str) -> El<'a> {
        exactly_one(css, self.0.select(&selector(css)).collect(), || {
            snippet(&self.0.html(), 400)
        })
    }

    /// Whether the element itself matches `css`.
    pub fn is(&self, css: &str) -> bool {
        selector(css).matches(&self.0)
    }

    /// The element's markup, shortened, for failure messages.
    pub fn snippet(&self) -> String {
        snippet(&self.0.html(), 400)
    }
}

impl std::fmt::Debug for El<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.snippet())
    }
}
