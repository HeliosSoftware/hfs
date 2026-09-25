//! The one grammar, precision range and comparator mapping for date search
//! values (`date`, `dateTime`, `instant` parameters and `_lastUpdated`).
//!
//! The date analogue of [`super::range`]. Every backend used to parse date
//! search values itself and each disagreed on what was a date: PostgreSQL
//! substituted the current time for a value it could not parse (#1289),
//! Elasticsearch substituted the year 2000 (#1293), SQLite let `datetime()`
//! roll `2024-02-30` over to March (#1295), and only MongoDB returned an error.
//! They disagreed on ranges too: a second-precision `eq` matched a stored
//! `…:00.123` on SQLite and Elasticsearch but not on MongoDB or PostgreSQL
//! (#1297). This module is the single answer to both questions.
//!
//! # Grammar
//!
//! The FHIR *search* grammar, which is wider than the `dateTime` datatype
//! regex in exactly one place — seconds are optional
//! (<https://hl7.org/fhir/R4/search.html#date>: *"minutes SHALL be present if
//! an hour is present … Time can consist of hours and minutes with no seconds,
//! unlike the XML Schema dateTime type"*):
//!
//! ```text
//! YYYY | YYYY-MM | YYYY-MM-DD | YYYY-MM-DDThh:mm[:ss[.f+]][Z|(+|-)hh:mm]
//! ```
//!
//! - year `0001`–`9999`, month `01`–`12`, day valid for the month (leap years
//!   included), hour `00`–`23` (`24:00` is not allowed), minute `00`–`59`,
//!   second `00`–`60`;
//! - a leap second (`:60`) is read as the first instant of the next second, so
//!   no backend ever sees a `:60`;
//! - a zone is `Z` or `±hh:mm` up to `14:00`; a value without one is UTC, as
//!   is a date-only value;
//! - an hour without minutes (`T10`), lower-case `t`/`z`, and anything after
//!   the zone are rejected. Surrounding whitespace is trimmed.
//!
//! # `+` decoded to a space
//!
//! `application/x-www-form-urlencoded` decoding turns an unencoded `+` into a
//! space, so `date=2013-04-05T18:50:00+05:30` arrives as
//! `2013-04-05T18:50:00 05:30` (#1296). The grammar has no legal space, so a
//! space in the zone-sign position can only be a decoded `+` and is read as
//! one. This is safe *only* because the value is already known to be a date:
//! for a string or token a space is content. [`FhirDateValue::canonical`]
//! returns the repaired text; backends must use it, or the parsed instants,
//! and never the raw value.
//!
//! # Ranges and prefixes
//!
//! A search value is a half-open range `[s, e)` one unit of its own precision
//! wide — a year, a month, a day, a minute, a second, or `10⁻ⁿ` s for `n`
//! fraction digits.
//!
//! Every indexed date is a range `[ts, te)` too (#1391). A point value — a
//! `date`, `dateTime` or `instant` — covers one unit of its own precision, so
//! `birthDate: "2020"` is the whole of 2020. A `Period` (and a `Timing`'s
//! `repeat.boundsPeriod`) is one range from its `start` to the end of its
//! `end`, and a missing side is open: it is stored as [`OPEN_START`] or
//! [`open_end`], the limits of the supported years, which no search range
//! passes, so they behave as ±infinity. See [`indexed_range`].
//!
//! [`FhirDateValue::range_predicate`] implements the FHIR rules for a search
//! range against a target range:
//!
//! | prefix | match | [`RangePredicate`] groups |
//! |--------|-------|---------------------------|
//! | `eq` | `s ≤ ts ∧ te ≤ e` | `[ts ≥ s, te ≤ e]` |
//! | `ne` | `ts < s ∨ te > e` | `[ts < s] ∨ [te > e]` |
//! | `gt` | `te > e` | `[te > e]` |
//! | `lt` | `ts < s` | `[ts < s]` |
//! | `ge` | `te > e ∨ eq` | `[te > e] ∨ [ts ≥ s, te ≤ e]` |
//! | `le` | `ts < s ∨ eq` | `[ts < s] ∨ [ts ≥ s, te ≤ e]` |
//! | `sa` | `ts ≥ e` | `[ts ≥ e]` |
//! | `eb` | `te ≤ s` | `[te ≤ s]` |
//! | `ap` | overlap with `[s − m, e + m)` | `[ts < e + m, te > s − m]` |
//!
//! The `ap` margin `m` is [`FhirDateValue::approx_margin`], the same on every
//! backend.
//!
//! # Point comparisons
//!
//! `_lastUpdated` (an instant held in the resource row, not in the index) and
//! the date component of a composite parameter are still compared as a point
//! `t`, and [`FhirDateValue::predicate`] keeps that mapping:
//!
//! | prefix | match | [`DatePredicate`] |
//! |--------|-------|-------------------|
//! | `eq` | `start ≤ t < end` | `Within` |
//! | `ne` | `t < start ∨ t ≥ end` | `Outside` |
//! | `gt` / `sa` | `t ≥ end` | `AtOrAfter(end)` |
//! | `lt` / `eb` | `t < start` | `Before(start)` |
//! | `ge` | `t ≥ start` | `AtOrAfter(start)` |
//! | `le` | `t < end` | `Before(end)` |
//!
//! `ap` is absent there: [`FhirDateValue::predicate`] returns `None` for it,
//! and a point comparison builds its window from
//! [`FhirDateValue::approx_window`].

use chrono::{DateTime, Duration, Months, NaiveDate, TimeZone, Utc};

use super::converters::{DateEnd, IndexValue};
use crate::error::{SearchError, StorageError, StorageResult};
use crate::types::{
    DatePrecision, SearchModifier, SearchParamType, SearchParameter, SearchPrefix, SearchQuery,
};

/// The forms a date search value may take, for error messages.
const EXPECTED: &str =
    "expected YYYY, YYYY-MM, YYYY-MM-DD or YYYY-MM-DDThh:mm[:ss[.fff]][Z|(+|-)hh:mm]";

/// The precision a date search value was written at, which sets the width of
/// its range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateValuePrecision {
    /// `YYYY`.
    Year,
    /// `YYYY-MM`.
    Month,
    /// `YYYY-MM-DD`.
    Day,
    /// `…Thh:mm` — valid in search although not in the `dateTime` datatype.
    Minute,
    /// `…Thh:mm:ss`.
    Second,
    /// `…Thh:mm:ss.f+`, with the number of fraction digits supplied (1–9;
    /// digits past the ninth are ignored).
    Fraction(u8),
}

/// The finest instant a backend's date column can hold. A search range is
/// never narrower than this, or a finer-grained search value could not match
/// the stored value it was copied from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageResolution {
    /// Milliseconds: BSON dates, Elasticsearch `date`, SQLite's `%f`.
    Millis,
    /// Microseconds: PostgreSQL `TIMESTAMPTZ`.
    Micros,
}

impl StorageResolution {
    fn nanos(self) -> i64 {
        match self {
            StorageResolution::Millis => 1_000_000,
            StorageResolution::Micros => 1_000,
        }
    }
}

/// Why a value is not a date search value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateValueErrorReason {
    /// The value is empty.
    Empty,
    /// The value does not have the shape of a date at all.
    Syntax,
    /// Year `0000`.
    YearZero,
    /// Month outside `01`–`12`.
    MonthOutOfRange,
    /// Day that does not exist in the month (`2024-02-30`, `2023-02-29`).
    DayOutOfRange,
    /// An hour without minutes (`T10`).
    HourOnly,
    /// Hour outside `00`–`23`.
    HourOutOfRange,
    /// Minute outside `00`–`59`.
    MinuteOutOfRange,
    /// Second outside `00`–`60`.
    SecondOutOfRange,
    /// Zone offset beyond `±14:00`, or with minutes outside `00`–`59`.
    OffsetOutOfRange,
    /// A complete date followed by something else.
    TrailingInput,
    /// A valid value whose UTC instant falls outside the years 0001–9999.
    OutOfRange,
}

impl std::fmt::Display for DateValueErrorReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DateValueErrorReason::Empty => "the value is empty",
            DateValueErrorReason::Syntax => "not a date",
            DateValueErrorReason::YearZero => "year 0000 is not allowed",
            DateValueErrorReason::MonthOutOfRange => "month out of range",
            DateValueErrorReason::DayOutOfRange => "day out of range for month",
            DateValueErrorReason::HourOnly => "minutes are required when an hour is given",
            DateValueErrorReason::HourOutOfRange => "hour out of range",
            DateValueErrorReason::MinuteOutOfRange => "minute out of range",
            DateValueErrorReason::SecondOutOfRange => "second out of range",
            DateValueErrorReason::OffsetOutOfRange => "timezone offset out of range",
            DateValueErrorReason::TrailingInput => "unexpected characters after the date",
            DateValueErrorReason::OutOfRange => "outside the years 0001 to 9999",
        })
    }
}

/// A value that is not a FHIR date search value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "'{value}' is not a valid FHIR date/dateTime/instant ({reason}); {}",
    EXPECTED
)]
pub struct DateValueError {
    /// The rejected value, as received.
    pub value: String,
    /// What is wrong with it.
    pub reason: DateValueErrorReason,
}

/// A comparison against a stored point `t`. Only `>=` and `<` ever appear, so
/// a backend needs no other operator to translate one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePredicate {
    /// `ge ≤ t < lt`.
    Within {
        /// Inclusive lower bound.
        ge: DateTime<Utc>,
        /// Exclusive upper bound.
        lt: DateTime<Utc>,
    },
    /// `t < lt ∨ t ≥ ge`. Like every other prefix, this needs a `t`: a
    /// resource with no value for the parameter does not match.
    Outside {
        /// Exclusive upper bound of the part below the range.
        lt: DateTime<Utc>,
        /// Inclusive lower bound of the part above it.
        ge: DateTime<Utc>,
    },
    /// `t ≥ bound`.
    AtOrAfter(DateTime<Utc>),
    /// `t < bound`.
    Before(DateTime<Utc>),
}

impl DatePredicate {
    /// Whether a stored point satisfies the predicate.
    pub fn matches(&self, t: DateTime<Utc>) -> bool {
        match *self {
            DatePredicate::Within { ge, lt } => ge <= t && t < lt,
            DatePredicate::Outside { lt, ge } => t < lt || t >= ge,
            DatePredicate::AtOrAfter(bound) => t >= bound,
            DatePredicate::Before(bound) => t < bound,
        }
    }
}

/// A parsed date search value: the half-open UTC range `[start, end)` it
/// denotes, and the precision it was written at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FhirDateValue {
    /// Inclusive start of the range.
    pub start: DateTime<Utc>,
    /// Exclusive end of the range; always after `start`.
    pub end: DateTime<Utc>,
    /// The precision the value was written at.
    pub precision: DateValuePrecision,
    /// Whether a space in the zone-sign position was read as `+` (#1296).
    pub repaired_plus: bool,
    canonical: String,
}

impl FhirDateValue {
    /// Parses a date search value whose comparator prefix has already been
    /// removed.
    pub fn parse(raw: &str) -> Result<Self, DateValueError> {
        parse(raw.trim()).map_err(|reason| DateValueError {
            value: raw.to_string(),
            reason,
        })
    }

    /// The value as the client meant it: trimmed, with a form-decoded `+`
    /// restored.
    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    /// The range clamped to what a backend can store: `start` floored to the
    /// resolution, and never narrower than one unit of it.
    ///
    /// A client that echoes back a microsecond timestamp
    /// (`…:57.246958-08:00`) must still match the millisecond-truncated value
    /// a BSON or Elasticsearch date holds for it.
    pub fn range_at(&self, resolution: StorageResolution) -> (DateTime<Utc>, DateTime<Utc>) {
        let unit = resolution.nanos();
        let floor = |t: DateTime<Utc>| {
            t - Duration::nanoseconds(i64::from(t.timestamp_subsec_nanos()) % unit)
        };
        let start = floor(self.start);
        let end = floor(self.end).max(start + Duration::nanoseconds(unit));
        (start, end)
    }

    /// The comparison `prefix` makes against a stored point, per the table in
    /// the module docs.
    ///
    /// `None` for `ap`, whose window is each backend's own; it should be built
    /// around [`Self::range_at`].
    pub fn predicate(
        &self,
        prefix: SearchPrefix,
        resolution: StorageResolution,
    ) -> Option<DatePredicate> {
        let (start, end) = self.range_at(resolution);
        Some(match prefix {
            SearchPrefix::Eq => DatePredicate::Within { ge: start, lt: end },
            SearchPrefix::Ne => DatePredicate::Outside { lt: start, ge: end },
            SearchPrefix::Gt | SearchPrefix::Sa => DatePredicate::AtOrAfter(end),
            SearchPrefix::Lt | SearchPrefix::Eb => DatePredicate::Before(start),
            SearchPrefix::Ge => DatePredicate::AtOrAfter(start),
            SearchPrefix::Le => DatePredicate::Before(end),
            SearchPrefix::Ap => return None,
        })
    }

    /// The comparison `prefix` makes against a stored range `[ts, te)`, per
    /// the range table in the module docs. Every prefix has one, `ap`
    /// included: its window is [`Self::approx_window`], the same on every
    /// backend.
    pub fn range_predicate(
        &self,
        prefix: SearchPrefix,
        resolution: StorageResolution,
    ) -> RangePredicate {
        use RangeCondition::*;
        let (s, e) = self.range_at(resolution);
        let any_of = match prefix {
            SearchPrefix::Eq => vec![vec![StartAtOrAfter(s), EndAtOrBefore(e)]],
            SearchPrefix::Ne => vec![vec![StartBefore(s)], vec![EndAfter(e)]],
            SearchPrefix::Gt => vec![vec![EndAfter(e)]],
            SearchPrefix::Lt => vec![vec![StartBefore(s)]],
            SearchPrefix::Ge => vec![vec![EndAfter(e)], vec![StartAtOrAfter(s), EndAtOrBefore(e)]],
            SearchPrefix::Le => vec![
                vec![StartBefore(s)],
                vec![StartAtOrAfter(s), EndAtOrBefore(e)],
            ],
            SearchPrefix::Sa => vec![vec![StartAtOrAfter(e)]],
            SearchPrefix::Eb => vec![vec![EndAtOrBefore(s)]],
            SearchPrefix::Ap => {
                let (low, high) = self.approx_window(resolution);
                vec![vec![StartBefore(high), EndAfter(low)]]
            }
        };
        RangePredicate { any_of }
    }

    /// The window `ap` accepts an overlap with: the range widened on both
    /// sides by [`Self::approx_margin`], and kept inside the supported years.
    pub fn approx_window(&self, resolution: StorageResolution) -> (DateTime<Utc>, DateTime<Utc>) {
        let (s, e) = self.range_at(resolution);
        let margin = self.approx_margin();
        let low = margin
            .before(s)
            .unwrap_or_else(open_start)
            .max(open_start());
        let high = margin
            .after(e)
            .unwrap_or_else(|| open_end(resolution))
            .min(open_end(resolution));
        (low, high)
    }

    /// How far `ap` reaches past each side of the range: one unit of a
    /// date-only precision (a year, a month, a day), ten minutes for a minute
    /// and ten seconds for anything finer. The FHIR specification leaves the
    /// width to the server ("the recommended value … is 10% of the gap between
    /// now and the date"); a fixed window per precision is the same answer on
    /// every backend and for every query.
    pub fn approx_margin(&self) -> ApproxMargin {
        match self.precision {
            DateValuePrecision::Year => ApproxMargin::Months(12),
            DateValuePrecision::Month => ApproxMargin::Months(1),
            DateValuePrecision::Day => ApproxMargin::Fixed(Duration::days(1)),
            DateValuePrecision::Minute => ApproxMargin::Fixed(Duration::minutes(10)),
            DateValuePrecision::Second | DateValuePrecision::Fraction(_) => {
                ApproxMargin::Fixed(Duration::seconds(10))
            }
        }
    }
}

/// The width of the `ap` window on each side of a search range. Months are
/// kept apart from fixed durations because a month or a year is not a fixed
/// number of seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproxMargin {
    /// A number of calendar months.
    Months(u32),
    /// A fixed duration.
    Fixed(Duration),
}

impl ApproxMargin {
    /// `t` moved back by the margin, or `None` past the calendar's reach.
    pub fn before(self, t: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            ApproxMargin::Months(n) => t.checked_sub_months(Months::new(n)),
            ApproxMargin::Fixed(d) => t.checked_sub_signed(d),
        }
    }

    /// `t` moved forward by the margin, or `None` past the calendar's reach.
    pub fn after(self, t: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            ApproxMargin::Months(n) => t.checked_add_months(Months::new(n)),
            ApproxMargin::Fixed(d) => t.checked_add_signed(d),
        }
    }
}

/// One comparison against a stored range `[ts, te)`: a bound on the start
/// column or on the end column. The four cover every prefix, so a backend
/// needs `>=` and `<` for the start and `>` and `<=` for the end, nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeCondition {
    /// `ts ≥ bound`.
    StartAtOrAfter(DateTime<Utc>),
    /// `ts < bound`.
    StartBefore(DateTime<Utc>),
    /// `te > bound`.
    EndAfter(DateTime<Utc>),
    /// `te ≤ bound`.
    EndAtOrBefore(DateTime<Utc>),
}

impl RangeCondition {
    /// Whether a stored range satisfies the condition.
    pub fn matches(&self, ts: DateTime<Utc>, te: DateTime<Utc>) -> bool {
        match *self {
            RangeCondition::StartAtOrAfter(bound) => ts >= bound,
            RangeCondition::StartBefore(bound) => ts < bound,
            RangeCondition::EndAfter(bound) => te > bound,
            RangeCondition::EndAtOrBefore(bound) => te <= bound,
        }
    }
}

/// A comparison against a stored range `[ts, te)`, in disjunctive normal
/// form: it holds when every condition of at least one group holds. No prefix
/// needs more than two groups of two, and a backend translates any of them
/// with the same two nested loops (OR of ANDs).
///
/// Like [`DatePredicate`], it needs a stored range: a resource with no value
/// for the parameter does not match, `ne` included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangePredicate {
    /// The groups, OR-ed; the conditions inside a group are AND-ed. Never
    /// empty, and no group is empty.
    pub any_of: Vec<Vec<RangeCondition>>,
}

impl RangePredicate {
    /// Whether a stored range satisfies the predicate.
    pub fn matches(&self, ts: DateTime<Utc>, te: DateTime<Utc>) -> bool {
        self.any_of
            .iter()
            .any(|group| group.iter().all(|condition| condition.matches(ts, te)))
    }
}

/// The text a `Period` without a `start` is indexed with as its start: the
/// first instant of the supported years, which no search range starts before,
/// so it reads as "unbounded below" to every comparison (#1391).
pub const OPEN_START: &str = "0001-01-01T00:00:00Z";

/// The stored start of a range open below, as an instant: [`OPEN_START`].
pub fn open_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(1, 1, 1, 0, 0, 0)
        .single()
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// The stored end of a range open above: the last instant of the supported
/// years at `resolution`, which is where every search range is clamped, so
/// no search range ends after it (#1391).
///
/// A sentinel rather than NULL, so that `value_date_end` is always set on a
/// row indexed after #1391 and a missing end means only "indexed before it,
/// reindex" — never "open", which would match `gt` for any date.
pub fn open_end(resolution: StorageResolution) -> DateTime<Utc> {
    let last = Utc
        .with_ymd_and_hms(9999, 12, 31, 23, 59, 59)
        .single()
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
        + Duration::nanoseconds(999_999_999);
    last - Duration::nanoseconds(i64::from(last.timestamp_subsec_nanos()) % resolution.nanos())
}

/// Reads a date as it is stored in a resource. Only the text as written
/// counts: the search-side repairs (trimming, a space read as a form-decoded
/// `+`, #1296) do not apply to a resource, where a space is simply not part of
/// a date.
pub fn parse_stored_date(raw: &str) -> Option<FhirDateValue> {
    FhirDateValue::parse(raw)
        .ok()
        .filter(|parsed| parsed.canonical() == raw)
}

/// Where the stored range of a date index value ends — see
/// [`crate::search::DateEnd`] — given where it starts.
///
/// For a backend that read the start itself (some are more lenient than the
/// FHIR grammar about what they index): `start` is that instant and
/// `precision` the one the value carries. A point ends one unit of its
/// precision later, a `Period` at the end of its own `end` (so `end: "2020-06"`
/// ends on July 1st), and an open `Period` at [`open_end`].
///
/// `None` when a `Period`'s `end` is not a date, and the row must be skipped
/// like any other unparseable date: indexing it as open would over-match. The
/// result is never earlier than one unit past `start`, so a `Period` whose
/// `end` precedes its `start` (invalid FHIR, `per-1`) is still a non-empty
/// range starting where it says.
pub fn indexed_end(
    start: DateTime<Utc>,
    precision: DatePrecision,
    end: &DateEnd,
    resolution: StorageResolution,
) -> Option<DateTime<Utc>> {
    let unit = Duration::nanoseconds(resolution.nanos());
    let end = match end {
        DateEnd::Precision => match precision {
            DatePrecision::Year => start.checked_add_months(Months::new(12)),
            DatePrecision::Month => start.checked_add_months(Months::new(1)),
            DatePrecision::Day => start.checked_add_signed(Duration::days(1)),
            DatePrecision::Hour => start.checked_add_signed(Duration::hours(1)),
            DatePrecision::Minute => start.checked_add_signed(Duration::minutes(1)),
            DatePrecision::Second => start.checked_add_signed(Duration::seconds(1)),
            DatePrecision::Millisecond => start.checked_add_signed(Duration::milliseconds(1)),
        }
        .unwrap_or_else(|| open_end(resolution)),
        DateEnd::At(raw) => parse_stored_date(raw)?.range_at(resolution).1,
        DateEnd::Open => open_end(resolution),
    };
    Some(end.min(open_end(resolution)).max(start + unit))
}

/// The range `[start, end)` an index row stores for a date value, both ends
/// at `resolution`: the start of the value's own range, and the end
/// [`indexed_end`] gives it. A point's range is exactly the range a search
/// for the same text covers.
///
/// `None` for anything but [`IndexValue::Date`], and when either end is not a
/// date the FHIR grammar accepts as written; a backend that indexes wider
/// than the grammar falls back to [`indexed_end`] with its own start.
pub fn indexed_range(
    value: &IndexValue,
    resolution: StorageResolution,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let IndexValue::Date {
        value,
        precision,
        end,
    } = value
    else {
        return None;
    };
    let parsed = parse_stored_date(value)?;
    let (start, point_end) = parsed.range_at(resolution);
    let end = match end {
        // The parsed range is exact for every precision, fraction digits
        // included, where `DatePrecision` only knows "millisecond".
        DateEnd::Precision => point_end.min(open_end(resolution)),
        other => indexed_end(start, *precision, other, resolution)?,
    };
    Some((start, end))
}

/// A cursor over the ASCII bytes of a value.
struct Scanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn at_end(&self) -> bool {
        self.pos == self.bytes.len()
    }

    /// Consumes `byte` if it is next.
    fn eat(&mut self, byte: u8) -> bool {
        let hit = self.peek() == Some(byte);
        if hit {
            self.pos += 1;
        }
        hit
    }

    /// Consumes exactly `width` digits.
    fn digits(&mut self, width: usize) -> Option<u32> {
        let field = self.bytes.get(self.pos..self.pos + width)?;
        if !field.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.pos += width;
        Some(
            field
                .iter()
                .fold(0, |n, digit| n * 10 + u32::from(digit - b'0')),
        )
    }
}

fn parse(text: &str) -> Result<FhirDateValue, DateValueErrorReason> {
    use DateValueErrorReason as Reason;

    if text.is_empty() {
        return Err(Reason::Empty);
    }
    let mut scan = Scanner {
        bytes: text.as_bytes(),
        pos: 0,
    };

    let year = scan.digits(4).ok_or(Reason::Syntax)?;
    if year == 0 {
        return Err(Reason::YearZero);
    }
    let year = year as i32;
    if scan.at_end() {
        return finish(text, year, 1, 1, DateValuePrecision::Year);
    }

    if !scan.eat(b'-') {
        return Err(Reason::Syntax);
    }
    let month = scan.digits(2).ok_or(Reason::Syntax)?;
    if !(1..=12).contains(&month) {
        return Err(Reason::MonthOutOfRange);
    }
    if scan.at_end() {
        return finish(text, year, month, 1, DateValuePrecision::Month);
    }

    if !scan.eat(b'-') {
        return Err(Reason::Syntax);
    }
    let day = scan.digits(2).ok_or(Reason::Syntax)?;
    let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
        return Err(Reason::DayOutOfRange);
    };
    if scan.at_end() {
        return finish(text, year, month, day, DateValuePrecision::Day);
    }

    // A time can only follow a full date, and only after an upper-case `T`.
    if !scan.eat(b'T') {
        return Err(match scan.peek() {
            Some(b't' | b' ') => Reason::Syntax,
            _ => Reason::TrailingInput,
        });
    }
    let hour = scan.digits(2).ok_or(Reason::Syntax)?;
    if hour > 23 {
        return Err(Reason::HourOutOfRange);
    }
    if !scan.eat(b':') {
        return Err(if matches!(scan.peek(), None | Some(b'Z' | b'+' | b'-')) {
            Reason::HourOnly
        } else {
            Reason::Syntax
        });
    }
    let minute = scan.digits(2).ok_or(Reason::Syntax)?;
    if minute > 59 {
        return Err(Reason::MinuteOutOfRange);
    }

    let mut precision = DateValuePrecision::Minute;
    let mut second = 0;
    let mut nanos: u32 = 0;
    if scan.eat(b':') {
        second = scan.digits(2).ok_or(Reason::Syntax)?;
        if second > 60 {
            return Err(Reason::SecondOutOfRange);
        }
        precision = DateValuePrecision::Second;
        if scan.eat(b'.') {
            let mut count: u32 = 0;
            while let Some(digit) = scan.peek().filter(u8::is_ascii_digit) {
                // Digits past the ninth are below any resolution kept here.
                if count < 9 {
                    nanos = nanos * 10 + u32::from(digit - b'0');
                }
                count += 1;
                scan.pos += 1;
            }
            if count == 0 {
                return Err(Reason::Syntax);
            }
            let kept = count.min(9);
            nanos *= 10u32.pow(9 - kept);
            precision = DateValuePrecision::Fraction(kept as u8);
        }
    }

    // Zone: `Z`, a signed `hh:mm`, or nothing (UTC). A space where the sign
    // belongs is a `+` that form-decoding turned into a space (#1296).
    let mut repaired_plus = false;
    let mut offset_seconds: i64 = 0;
    if !scan.at_end() && !scan.eat(b'Z') {
        let sign = match scan.peek() {
            Some(b'+') => 1,
            Some(b'-') => -1,
            Some(b' ') => {
                repaired_plus = true;
                1
            }
            Some(b'z') => return Err(Reason::Syntax),
            _ => return Err(Reason::TrailingInput),
        };
        scan.pos += 1;
        let zone_hour = scan.digits(2).ok_or(Reason::Syntax)?;
        if !scan.eat(b':') {
            return Err(Reason::Syntax);
        }
        let zone_minute = scan.digits(2).ok_or(Reason::Syntax)?;
        if zone_minute > 59 || zone_hour > 14 || (zone_hour == 14 && zone_minute != 0) {
            return Err(Reason::OffsetOutOfRange);
        }
        offset_seconds = sign * i64::from(zone_hour * 3600 + zone_minute * 60);
    }
    if !scan.at_end() {
        return Err(Reason::TrailingInput);
    }

    // `:60` is built as `:59` plus a second, which lands on the first instant
    // of the next second without ever constructing a leap-second time.
    let local = date
        .and_hms_nano_opt(hour, minute, second.min(59), nanos)
        .ok_or(Reason::Syntax)?;
    let leap = Duration::seconds(i64::from(second == 60));
    let start = Utc.from_utc_datetime(&local) + leap - Duration::seconds(offset_seconds);
    let width = match precision {
        DateValuePrecision::Minute => Duration::seconds(60),
        DateValuePrecision::Fraction(digits) => {
            Duration::nanoseconds(10i64.pow(9 - u32::from(digits)))
        }
        _ => Duration::seconds(1),
    };

    let canonical = if repaired_plus {
        text.replacen(' ', "+", 1)
    } else {
        text.to_string()
    };
    in_supported_years(FhirDateValue {
        start,
        end: start + width,
        precision,
        repaired_plus,
        canonical,
    })
}

/// Builds a date-only value: a UTC year, month or day.
fn finish(
    text: &str,
    year: i32,
    month: u32,
    day: u32,
    precision: DateValuePrecision,
) -> Result<FhirDateValue, DateValueErrorReason> {
    let midnight = |y: i32, m: u32, d: u32| {
        NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .map(|naive| Utc.from_utc_datetime(&naive))
            .ok_or(DateValueErrorReason::Syntax)
    };
    let start = midnight(year, month, day)?;
    let end = match precision {
        DateValuePrecision::Year => midnight(year + 1, 1, 1)?,
        DateValuePrecision::Month if month == 12 => midnight(year + 1, 1, 1)?,
        DateValuePrecision::Month => midnight(year, month + 1, 1)?,
        _ => start + Duration::days(1),
    };
    in_supported_years(FhirDateValue {
        start,
        end,
        precision,
        repaired_plus: false,
        canonical: text.to_string(),
    })
}

/// Keeps every bound a backend will format inside the four-digit years: an
/// offset can push `0001-01-01T00:00:00+14:00` or `9999-12-31T23:59:59-14:00`
/// outside them, and the range of `9999` ends in the year 10000.
fn in_supported_years(mut value: FhirDateValue) -> Result<FhirDateValue, DateValueErrorReason> {
    use chrono::Datelike;
    if !(1..=9999).contains(&value.start.year()) {
        return Err(DateValueErrorReason::OutOfRange);
    }
    let last = Utc
        .with_ymd_and_hms(9999, 12, 31, 23, 59, 59)
        .single()
        .ok_or(DateValueErrorReason::OutOfRange)?
        + Duration::nanoseconds(999_999_999);
    value.end = value.end.min(last);
    if value.end <= value.start {
        return Err(DateValueErrorReason::OutOfRange);
    }
    Ok(value)
}

/// Rejects a query carrying a date search value that is not a date.
///
/// Call this beside [`super::reject_unsupported_metadata_modifier`], from every
/// entry point that builds a backend query. Every `SearchQuery` reaches one —
/// REST searches, conditional operations, batch entries, compartment and
/// `$everything` searches, and the per-hop queries the chain resolver issues
/// for chained and `_has` terminals — so an invalid date is an error on every
/// path, not only the ones a REST extractor happened to cover.
pub fn validate_date_values(query: &SearchQuery) -> StorageResult<()> {
    for param in &query.parameters {
        validate_date_parameter(param).map_err(StorageError::Search)?;
    }
    Ok(())
}

/// Rejects a parameter carrying a date search value that is not a date.
///
/// Skipped for `:missing`, whose value is a boolean, and for a chained
/// parameter, whose value belongs to the chain's last link; the chain resolver
/// replaces it with a terminal query that is validated in its own right.
pub fn validate_date_parameter(param: &SearchParameter) -> Result<(), SearchError> {
    if matches!(param.modifier, Some(SearchModifier::Missing)) || !param.chain.is_empty() {
        return Ok(());
    }
    let invalid = |value: &str, error: DateValueError| SearchError::InvalidDateValue {
        param: param.name.clone(),
        value: value.to_string(),
        reason: error.to_string(),
    };

    // `_lastUpdated` is matched by name as well: a caller that built the
    // parameter by hand may not have typed it.
    if param.param_type == SearchParamType::Date || param.name == "_lastUpdated" {
        for value in &param.values {
            FhirDateValue::parse(&value.value).map_err(|e| invalid(&value.value, e))?;
        }
    } else if param.param_type == SearchParamType::Composite {
        // A composite value is `$`-joined, one part per component. A value
        // with the wrong number of parts is the backend's to reject.
        for value in &param.values {
            let parts = value.value.split('$');
            for (part, component) in parts.zip(&param.components) {
                if component.param_type == SearchParamType::Date {
                    let (_, date) = SearchPrefix::extract(part);
                    FhirDateValue::parse(date).map_err(|e| invalid(&value.value, e))?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChainedParameter, CompositeSearchComponent, SearchValue};

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn parsed(text: &str) -> FhirDateValue {
        FhirDateValue::parse(text).unwrap_or_else(|e| panic!("{text} should parse: {e}"))
    }

    #[test]
    fn valid_values_and_their_ranges() {
        use DateValuePrecision::*;
        for (input, precision, start, end) in [
            ("2013", Year, "2013-01-01T00:00:00Z", "2014-01-01T00:00:00Z"),
            (
                "2013-04",
                Month,
                "2013-04-01T00:00:00Z",
                "2013-05-01T00:00:00Z",
            ),
            // December rolls into the next year.
            (
                "2013-12",
                Month,
                "2013-12-01T00:00:00Z",
                "2014-01-01T00:00:00Z",
            ),
            (
                "2013-04-05",
                Day,
                "2013-04-05T00:00:00Z",
                "2013-04-06T00:00:00Z",
            ),
            (
                "2024-02-29",
                Day,
                "2024-02-29T00:00:00Z",
                "2024-03-01T00:00:00Z",
            ),
            // Minute precision: valid in search, unlike the dateTime datatype.
            (
                "2013-04-05T09:20",
                Minute,
                "2013-04-05T09:20:00Z",
                "2013-04-05T09:21:00Z",
            ),
            (
                "2013-04-05T09:20-04:00",
                Minute,
                "2013-04-05T13:20:00Z",
                "2013-04-05T13:21:00Z",
            ),
            (
                "2013-04-05T09:20:00",
                Second,
                "2013-04-05T09:20:00Z",
                "2013-04-05T09:20:01Z",
            ),
            (
                "2013-04-05T09:20:00Z",
                Second,
                "2013-04-05T09:20:00Z",
                "2013-04-05T09:20:01Z",
            ),
            (
                "2013-04-05T09:20:00-04:00",
                Second,
                "2013-04-05T13:20:00Z",
                "2013-04-05T13:20:01Z",
            ),
            (
                "2013-04-05T18:50:00+05:30",
                Second,
                "2013-04-05T13:20:00Z",
                "2013-04-05T13:20:01Z",
            ),
            (
                "2013-04-05T09:20:00-00:00",
                Second,
                "2013-04-05T09:20:00Z",
                "2013-04-05T09:20:01Z",
            ),
            (
                "2013-04-05T23:20:00+14:00",
                Second,
                "2013-04-05T09:20:00Z",
                "2013-04-05T09:20:01Z",
            ),
            (
                "2021-11-10T16:48:57.246958-08:00",
                Fraction(6),
                "2021-11-11T00:48:57.246958Z",
                "2021-11-11T00:48:57.246959Z",
            ),
            (
                "2013-04-05T09:20:00.5Z",
                Fraction(1),
                "2013-04-05T09:20:00.5Z",
                "2013-04-05T09:20:00.6Z",
            ),
            (
                "2013-04-05T09:20:00.804Z",
                Fraction(3),
                "2013-04-05T09:20:00.804Z",
                "2013-04-05T09:20:00.805Z",
            ),
            // Digits past the ninth are dropped, not rounded.
            (
                "2013-04-05T09:20:00.1234567899Z",
                Fraction(9),
                "2013-04-05T09:20:00.123456789Z",
                "2013-04-05T09:20:00.123456790Z",
            ),
            // A leap second is the first instant of the next second.
            (
                "2016-12-31T23:59:60Z",
                Second,
                "2017-01-01T00:00:00Z",
                "2017-01-01T00:00:01Z",
            ),
            // Surrounding whitespace is not part of the value.
            (
                " 2013-04-05 ",
                Day,
                "2013-04-05T00:00:00Z",
                "2013-04-06T00:00:00Z",
            ),
        ] {
            let value = parsed(input);
            assert_eq!(value.precision, precision, "precision of {input}");
            assert_eq!(value.start, utc(start), "start of {input}");
            assert_eq!(value.end, utc(end), "end of {input}");
            assert!(!value.repaired_plus, "{input} needed no repair");
            assert_eq!(value.canonical(), input.trim());
        }
    }

    #[test]
    fn invalid_values_and_why() {
        use DateValueErrorReason::*;
        for (input, reason) in [
            ("", Empty),
            ("   ", Empty),
            ("not-a-date", Syntax),
            ("abcd", Syntax),
            ("true", Syntax),
            ("now", Syntax),
            ("2024-1x", Syntax),
            ("2024-1", Syntax),
            ("2024-1-5", Syntax),
            ("20240115", Syntax),
            ("0000", YearZero),
            ("0000-01-01", YearZero),
            ("2024-13-45", MonthOutOfRange),
            ("2024-00", MonthOutOfRange),
            ("2024-02-30", DayOutOfRange),
            ("2023-02-29", DayOutOfRange),
            ("2024-04-31", DayOutOfRange),
            ("2024-01-00", DayOutOfRange),
            ("2013-04-05T", Syntax),
            ("2013-04-05T10", HourOnly),
            ("2013-04-05T10Z", HourOnly),
            ("2013-04-05T10-04:00", HourOnly),
            ("2013-04T10:00", Syntax),
            ("2013-04-05T25:00:00Z", HourOutOfRange),
            ("2013-04-05T24:00:00Z", HourOutOfRange),
            ("2013-04-05T09:60:00Z", MinuteOutOfRange),
            ("2013-04-05T09:20:61Z", SecondOutOfRange),
            ("2013-04-05T09:20:00.Z", Syntax),
            // A fraction needs seconds to belong to.
            ("2013-04-05T09:20.5", TrailingInput),
            ("2013-04-05T09:20:00+14:30", OffsetOutOfRange),
            ("2013-04-05T09:20:00+15:00", OffsetOutOfRange),
            ("2013-04-05T09:20:00-04:60", OffsetOutOfRange),
            ("2013-04-05T09:20:00-99", Syntax),
            ("2013-04-05T09:20:00+05:3", Syntax),
            ("2013-04-05T09:20:00+0530", Syntax),
            // Lower case is not in the grammar.
            ("2013-04-05T09:20:00z", Syntax),
            ("2013-04-05t09:20:00Z", Syntax),
            ("2013-04-05 09:20:00", Syntax),
            // Not the shape a decoded `+` leaves behind.
            ("2013-04-05T09:20:00 5:30", Syntax),
            ("2013-04-05T09:20:00  05:30", Syntax),
            ("2013-04-05T09:20:00Z05:30", TrailingInput),
            ("2013-04-05T09:20:00ZZ", TrailingInput),
            // A comparator prefix is the caller's to remove.
            ("ge1980-01-01", Syntax),
            ("T25:00:00Z", Syntax),
            ("-2013", Syntax),
            ("２０１３", Syntax),
            // Valid text, but the instant leaves the supported years.
            ("0001-01-01T00:00:00+14:00", OutOfRange),
            ("9999-12-31T23:59:59-14:00", OutOfRange),
        ] {
            match FhirDateValue::parse(input) {
                Ok(value) => panic!("{input:?} parsed as {value:?}"),
                Err(error) => {
                    assert_eq!(error.reason, reason, "reason for {input:?}");
                    assert_eq!(error.value, input);
                }
            }
        }
    }

    #[test]
    fn trailing_space_after_a_repaired_zone_is_trimmed_first() {
        // Trimming happens before the scan, so this is the repaired form.
        assert!(parsed("2013-04-05T09:20:00 05:30 ").repaired_plus);
    }

    #[test]
    fn a_space_in_the_sign_position_is_a_decoded_plus() {
        for (decoded, meant) in [
            ("2013-04-05T18:50:00 05:30", "2013-04-05T18:50:00+05:30"),
            ("2013-04-05T18:50 05:30", "2013-04-05T18:50+05:30"),
            (
                "2013-04-05T18:50:00.123 05:30",
                "2013-04-05T18:50:00.123+05:30",
            ),
        ] {
            let repaired = parsed(decoded);
            let literal = parsed(meant);
            assert!(repaired.repaired_plus);
            assert_eq!(repaired.canonical(), meant);
            assert_eq!(repaired.start, literal.start);
            assert_eq!(repaired.end, literal.end);
            assert_eq!(repaired.precision, literal.precision);
        }
        // The restored offset is range-checked like any other.
        assert_eq!(
            FhirDateValue::parse("2013-04-05T18:50:00 15:00")
                .unwrap_err()
                .reason,
            DateValueErrorReason::OffsetOutOfRange
        );
    }

    #[test]
    fn the_last_year_has_a_representable_end() {
        let value = parsed("9999");
        assert_eq!(value.start, utc("9999-01-01T00:00:00Z"));
        assert_eq!(value.end, utc("9999-12-31T23:59:59.999999999Z"));
    }

    #[test]
    fn range_is_clamped_to_the_storage_resolution() {
        // Inferno echoes microseconds; BSON and Elasticsearch hold milliseconds.
        let micros = parsed("2021-11-10T16:48:57.246958-08:00");
        assert_eq!(
            micros.range_at(StorageResolution::Millis),
            (
                utc("2021-11-11T00:48:57.246Z"),
                utc("2021-11-11T00:48:57.247Z")
            )
        );
        assert_eq!(
            micros.range_at(StorageResolution::Micros),
            (micros.start, micros.end)
        );

        // Nanoseconds are finer than PostgreSQL's microseconds too.
        assert_eq!(
            parsed("2013-04-05T09:20:00.123456789Z").range_at(StorageResolution::Micros),
            (
                utc("2013-04-05T09:20:00.123456Z"),
                utc("2013-04-05T09:20:00.123457Z")
            )
        );

        // A range already wider than the resolution is left alone.
        for input in [
            "2013",
            "2013-04-05",
            "2013-04-05T09:20",
            "2013-04-05T09:20:00.5Z",
        ] {
            let value = parsed(input);
            for resolution in [StorageResolution::Millis, StorageResolution::Micros] {
                assert_eq!(
                    value.range_at(resolution),
                    (value.start, value.end),
                    "{input}"
                );
            }
        }
    }

    #[test]
    fn every_prefix_maps_to_its_predicate() {
        let value = parsed("2013-04-05T23:30:00-04:00");
        let (s, e) = (utc("2013-04-06T03:30:00Z"), utc("2013-04-06T03:30:01Z"));
        for (prefix, expected) in [
            (SearchPrefix::Eq, DatePredicate::Within { ge: s, lt: e }),
            (SearchPrefix::Ne, DatePredicate::Outside { lt: s, ge: e }),
            (SearchPrefix::Gt, DatePredicate::AtOrAfter(e)),
            (SearchPrefix::Sa, DatePredicate::AtOrAfter(e)),
            (SearchPrefix::Lt, DatePredicate::Before(s)),
            (SearchPrefix::Eb, DatePredicate::Before(s)),
            (SearchPrefix::Ge, DatePredicate::AtOrAfter(s)),
            (SearchPrefix::Le, DatePredicate::Before(e)),
        ] {
            assert_eq!(
                value.predicate(prefix, StorageResolution::Micros),
                Some(expected),
                "{prefix}"
            );
        }
        // `ap` stays with the backend.
        assert_eq!(
            value.predicate(SearchPrefix::Ap, StorageResolution::Micros),
            None
        );
    }

    #[test]
    fn second_precision_covers_a_stored_fraction() {
        // #1297: the stored value sits inside the searched second.
        let stored = utc("2013-04-05T23:30:00.123-04:00");
        let value = parsed("2013-04-05T23:30:00-04:00");
        let hit = |prefix| {
            value
                .predicate(prefix, StorageResolution::Millis)
                .unwrap()
                .matches(stored)
        };
        assert!(hit(SearchPrefix::Eq));
        assert!(hit(SearchPrefix::Ge));
        assert!(hit(SearchPrefix::Le));
        assert!(!hit(SearchPrefix::Ne));
        assert!(!hit(SearchPrefix::Gt));
        assert!(!hit(SearchPrefix::Lt));
        assert!(!hit(SearchPrefix::Sa));
        assert!(!hit(SearchPrefix::Eb));
    }

    fn range_of(start: Option<&str>, end: Option<&str>) -> (DateTime<Utc>, DateTime<Utc>) {
        let value = IndexValue::date_range(start, end).expect("a bound");
        indexed_range(&value, StorageResolution::Millis).expect("a range")
    }

    fn point_range(value: &str) -> (DateTime<Utc>, DateTime<Utc>) {
        indexed_range(&IndexValue::date(value), StorageResolution::Millis).expect("a range")
    }

    /// #1391: the FHIR table for a search range against a target range, every
    /// prefix against Periods that sit inside, across, before and after the
    /// searched year, and against open-ended ones.
    #[test]
    fn every_prefix_against_a_range_target() {
        let search = parsed("2020");
        let targets = [
            // Starts before 2020 and ends inside it: `eq` used to match on the end.
            ("across start", range_of(Some("2019-06"), Some("2020-03"))),
            ("inside", range_of(Some("2020-02-01"), Some("2020-05"))),
            ("before", range_of(Some("2018"), Some("2018-12-31"))),
            ("after", range_of(Some("2021-02"), Some("2021-03"))),
            // Covers the whole year: `sa2020` used to match on the end alone.
            ("covering", range_of(Some("2019-06"), Some("2021-03"))),
            ("open end", range_of(Some("2019-06-01"), None)),
            ("open start", range_of(None, Some("2021-03"))),
            ("point inside", point_range("2020-05-15")),
        ];
        // One column per target above, in order.
        #[rustfmt::skip]
        let expected = [
            (SearchPrefix::Eq, [false, true,  false, false, false, false, false, true ]),
            (SearchPrefix::Ne, [true,  false, true,  true,  true,  true,  true,  false]),
            (SearchPrefix::Gt, [false, false, false, true,  true,  true,  true,  false]),
            (SearchPrefix::Lt, [true,  false, true,  false, true,  true,  true,  false]),
            (SearchPrefix::Ge, [false, true,  false, true,  true,  true,  true,  true ]),
            (SearchPrefix::Le, [true,  true,  true,  false, true,  true,  true,  true ]),
            (SearchPrefix::Sa, [false, false, false, true,  false, false, false, false]),
            (SearchPrefix::Eb, [false, false, true,  false, false, false, false, false]),
            // The window is 2019 to 2021; "before" ends exactly where it starts.
            (SearchPrefix::Ap, [true,  true,  false, true,  true,  true,  true,  true ]),
        ];
        for (prefix, row) in expected {
            let predicate = search.range_predicate(prefix, StorageResolution::Millis);
            for ((name, (ts, te)), want) in targets.iter().zip(row) {
                assert_eq!(predicate.matches(*ts, *te), want, "{prefix} against {name}");
            }
        }
    }

    #[test]
    fn range_predicates_have_the_documented_shape() {
        use RangeCondition::*;
        let value = parsed("2013-04-05T23:30:00-04:00");
        let (s, e) = (utc("2013-04-06T03:30:00Z"), utc("2013-04-06T03:30:01Z"));
        let contained = vec![StartAtOrAfter(s), EndAtOrBefore(e)];
        for (prefix, any_of) in [
            (SearchPrefix::Eq, vec![contained.clone()]),
            (
                SearchPrefix::Ne,
                vec![vec![StartBefore(s)], vec![EndAfter(e)]],
            ),
            (SearchPrefix::Gt, vec![vec![EndAfter(e)]]),
            (SearchPrefix::Lt, vec![vec![StartBefore(s)]]),
            (SearchPrefix::Ge, vec![vec![EndAfter(e)], contained.clone()]),
            (
                SearchPrefix::Le,
                vec![vec![StartBefore(s)], contained.clone()],
            ),
            (SearchPrefix::Sa, vec![vec![StartAtOrAfter(e)]]),
            (SearchPrefix::Eb, vec![vec![EndAtOrBefore(s)]]),
            (
                SearchPrefix::Ap,
                vec![vec![
                    StartBefore(utc("2013-04-06T03:30:11Z")),
                    EndAfter(utc("2013-04-06T03:29:50Z")),
                ]],
            ),
        ] {
            assert_eq!(
                value.range_predicate(prefix, StorageResolution::Micros),
                RangePredicate { any_of },
                "{prefix}"
            );
        }
    }

    /// A point value's stored range is exactly the range a search for the same
    /// text covers, so `eq` with the stored text always finds it.
    #[test]
    fn a_point_is_stored_as_its_own_precision_range() {
        for (stored, start, end) in [
            ("2020", "2020-01-01T00:00:00Z", "2021-01-01T00:00:00Z"),
            ("2020-12", "2020-12-01T00:00:00Z", "2021-01-01T00:00:00Z"),
            ("2020-06-15", "2020-06-15T00:00:00Z", "2020-06-16T00:00:00Z"),
            (
                "2020-06-15T10:00:00+02:00",
                "2020-06-15T08:00:00Z",
                "2020-06-15T08:00:01Z",
            ),
        ] {
            assert_eq!(point_range(stored), (utc(start), utc(end)), "{stored}");
            let search =
                parsed(stored).range_predicate(SearchPrefix::Eq, StorageResolution::Millis);
            let (ts, te) = point_range(stored);
            assert!(search.matches(ts, te), "eq{stored}");
        }
        // Finer than the search: a whole stored day is not inside one minute.
        let (ts, te) = point_range("2020-01-01");
        assert!(
            !parsed("2020-01-01T00:00")
                .range_predicate(SearchPrefix::Eq, StorageResolution::Millis)
                .matches(ts, te)
        );
    }

    #[test]
    fn a_period_ends_at_the_end_of_its_own_end() {
        assert_eq!(
            range_of(Some("2020-01-15"), Some("2020-06")),
            (utc("2020-01-15T00:00:00Z"), utc("2020-07-01T00:00:00Z"))
        );
        assert_eq!(
            range_of(Some("2020-01-15T10:00:00Z"), Some("2020-01-15T12:30:00Z")),
            (utc("2020-01-15T10:00:00Z"), utc("2020-01-15T12:30:01Z"))
        );
        // `end` before `start` (invalid, `per-1`) still stores a non-empty
        // range starting where the Period says.
        let (ts, te) = range_of(Some("2020-06-01"), Some("2020-01-01"));
        assert_eq!(ts, utc("2020-06-01T00:00:00Z"));
        assert_eq!(te, ts + Duration::milliseconds(1));
    }

    #[test]
    fn open_ends_are_stored_as_the_supported_limits() {
        assert_eq!(
            range_of(Some("2020"), None),
            (
                utc("2020-01-01T00:00:00Z"),
                open_end(StorageResolution::Millis)
            )
        );
        assert_eq!(
            range_of(None, Some("2020")),
            (open_start(), utc("2021-01-01T00:00:00Z"))
        );
        assert_eq!(open_start(), utc(OPEN_START));
        assert_eq!(
            open_end(StorageResolution::Millis),
            utc("9999-12-31T23:59:59.999Z")
        );
        assert_eq!(
            open_end(StorageResolution::Micros),
            utc("9999-12-31T23:59:59.999999Z")
        );

        // No search range passes them, so they read as unbounded.
        for resolution in [StorageResolution::Millis, StorageResolution::Micros] {
            let (s, _) = parsed("0001").range_at(resolution);
            let (_, e) = parsed("9999").range_at(resolution);
            assert_eq!(s, open_start());
            assert_eq!(e, open_end(resolution));
        }
        let (ts, te) = range_of(Some("2019-06-01"), None);
        assert!(
            parsed("2030")
                .range_predicate(SearchPrefix::Gt, StorageResolution::Millis)
                .matches(ts, te)
        );
        let (ts, te) = range_of(None, Some("2020"));
        assert!(
            parsed("1900")
                .range_predicate(SearchPrefix::Lt, StorageResolution::Millis)
                .matches(ts, te)
        );
    }

    #[test]
    fn indexed_range_refuses_what_it_cannot_read() {
        let res = StorageResolution::Millis;
        assert_eq!(indexed_range(&IndexValue::string("2020"), res), None);
        assert_eq!(indexed_range(&IndexValue::date("not-a-date"), res), None);
        // The stored text counts as written: no search-side repairs.
        assert_eq!(indexed_range(&IndexValue::date(" 2020"), res), None);
        let bad_end = IndexValue::Date {
            value: "2020".to_string(),
            precision: DatePrecision::Year,
            end: DateEnd::At("2020-02-30".to_string()),
        };
        assert_eq!(indexed_range(&bad_end, res), None);
    }

    /// For a backend that read the start itself, more leniently than the
    /// grammar: the end follows the precision the value carries.
    #[test]
    fn indexed_end_from_a_backend_start() {
        let res = StorageResolution::Millis;
        let start = utc("2020-02-29T00:00:00Z");
        for (precision, end) in [
            (DatePrecision::Year, "2021-02-28T00:00:00Z"),
            (DatePrecision::Month, "2020-03-29T00:00:00Z"),
            (DatePrecision::Day, "2020-03-01T00:00:00Z"),
            (DatePrecision::Hour, "2020-02-29T01:00:00Z"),
            (DatePrecision::Minute, "2020-02-29T00:01:00Z"),
            (DatePrecision::Second, "2020-02-29T00:00:01Z"),
            (DatePrecision::Millisecond, "2020-02-29T00:00:00.001Z"),
        ] {
            assert_eq!(
                indexed_end(start, precision, &DateEnd::Precision, res),
                Some(utc(end)),
                "{precision:?}"
            );
        }
        assert_eq!(
            indexed_end(start, DatePrecision::Day, &DateEnd::Open, res),
            Some(open_end(res))
        );
        assert_eq!(
            indexed_end(
                start,
                DatePrecision::Day,
                &DateEnd::At("2020-03".into()),
                res
            ),
            Some(utc("2020-04-01T00:00:00Z"))
        );
        assert_eq!(
            indexed_end(start, DatePrecision::Day, &DateEnd::At("nope".into()), res),
            None
        );
    }

    #[test]
    fn approx_margin_follows_the_precision() {
        let res = StorageResolution::Millis;
        for (value, low, high) in [
            ("2020", "2019-01-01T00:00:00Z", "2022-01-01T00:00:00Z"),
            ("2020-03", "2020-02-01T00:00:00Z", "2020-05-01T00:00:00Z"),
            ("2020-03-15", "2020-03-14T00:00:00Z", "2020-03-17T00:00:00Z"),
            (
                "2020-03-15T10:00Z",
                "2020-03-15T09:50:00Z",
                "2020-03-15T10:11:00Z",
            ),
            (
                "2020-03-15T10:00:00Z",
                "2020-03-15T09:59:50Z",
                "2020-03-15T10:00:11Z",
            ),
        ] {
            assert_eq!(
                parsed(value).approx_window(res),
                (utc(low), utc(high)),
                "{value}"
            );
        }
        // Kept inside the supported years.
        assert_eq!(
            parsed("0001").approx_window(res),
            (open_start(), utc("0003-01-01T00:00:00Z"))
        );
        assert_eq!(parsed("9999").approx_window(res).1, open_end(res));
    }

    fn date_param(name: &str, value: &str) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        }
    }

    fn assert_invalid(query: &SearchQuery, expected_param: &str, expected_value: &str) {
        match validate_date_values(query) {
            Err(StorageError::Search(SearchError::InvalidDateValue { param, value, .. })) => {
                assert_eq!(param, expected_param);
                assert_eq!(value, expected_value);
            }
            other => panic!("expected InvalidDateValue, got {other:?}"),
        }
    }

    #[test]
    fn gate_rejects_invalid_date_values() {
        for value in [
            "not-a-date",
            "abcd",
            "2024-1x",
            "2024-13-45",
            "2024-02-30",
            "T25:00:00Z",
            "",
        ] {
            let query = SearchQuery::new("Procedure").with_parameter(date_param("date", value));
            assert_invalid(&query, "date", value);
            let query =
                SearchQuery::new("Procedure").with_parameter(date_param("_lastUpdated", value));
            assert_invalid(&query, "_lastUpdated", value);
        }
    }

    #[test]
    fn gate_checks_every_value_of_an_or_list() {
        let mut param = date_param("date", "2024-01-15");
        param.values.push(SearchValue::eq("nope"));
        let query = SearchQuery::new("Procedure").with_parameter(param);
        assert_invalid(&query, "date", "nope");
    }

    #[test]
    fn gate_accepts_valid_values() {
        let mut param = date_param("date", "2013-04-05T09:20");
        param.values.push(SearchValue::new(
            SearchPrefix::Ge,
            "2013-04-05T18:50:00 05:30",
        ));
        let query = SearchQuery::new("Procedure").with_parameter(param);
        assert!(validate_date_values(&query).is_ok());
    }

    #[test]
    fn gate_recognizes_last_updated_by_name() {
        let mut param = date_param("_lastUpdated", "garbage");
        param.param_type = SearchParamType::String;
        let query = SearchQuery::new("Patient").with_parameter(param);
        assert_invalid(&query, "_lastUpdated", "garbage");
    }

    #[test]
    fn gate_skips_missing_chains_and_other_types() {
        // `:missing` carries a boolean, not a date.
        let mut missing = date_param("date", "true");
        missing.modifier = Some(SearchModifier::Missing);
        // A chain's value belongs to its last link.
        let mut chained = date_param("subject", "ge1980-01-01");
        chained.chain = vec![ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "birthdate".to_string(),
        }];
        // Not a date parameter at all.
        let mut string = date_param("name", "2024-13-45");
        string.param_type = SearchParamType::String;

        let query = SearchQuery::new("Observation")
            .with_parameter(missing)
            .with_parameter(chained)
            .with_parameter(string);
        assert!(validate_date_values(&query).is_ok());
    }

    #[test]
    fn gate_checks_the_date_components_of_a_composite() {
        let composite = |value: &str| SearchParameter {
            name: "code-date".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Date,
                    param_name: "date".to_string(),
                },
            ],
        };

        for value in ["8867-4$2024-02-30", "8867-4$gtnot-a-date", "8867-4$"] {
            let query = SearchQuery::new("Observation").with_parameter(composite(value));
            assert_invalid(&query, "code-date", value);
        }
        // The token half is not a date, whatever it looks like; the date half
        // may carry a prefix.
        for value in ["2024-13-45$2024-02-29", "8867-4$ge2013-04-05T09:20"] {
            let query = SearchQuery::new("Observation").with_parameter(composite(value));
            assert!(validate_date_values(&query).is_ok(), "{value}");
        }
        // Without resolved components there is nothing to type the parts by.
        let mut untyped = composite("8867-4$not-a-date");
        untyped.components.clear();
        let query = SearchQuery::new("Observation").with_parameter(untyped);
        assert!(validate_date_values(&query).is_ok());
    }
}
