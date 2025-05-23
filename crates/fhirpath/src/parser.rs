use chumsky::Parser;
use chumsky::error::Simple;
use chumsky::prelude::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::fmt;
use std::str::FromStr;

/// Represents a literal value in FHIRPath
///
/// This enum represents all the different types of literal values that can appear
/// in a FHIRPath expression, including:
/// - Empty value (`{}`)
/// - Boolean values (true/false)
/// - String literals (e.g., 'text')
/// - Numeric values (integers and decimals)
/// - Date/time literals (date, datetime, time)
/// - Quantity values (numeric values with units)
///
/// These literals are used in the abstract syntax tree (AST) produced by the parser
/// and are later evaluated into concrete values during expression evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// The empty value, represented as `{}` in FHIRPath
    Null,
    /// Boolean true/false values
    Boolean(bool),
    /// String literals enclosed in single quotes
    String(String),
    /// Decimal numbers (with a decimal point)
    Number(Decimal),
    /// Integer numbers (without a decimal point)
    Integer(i64),
    /// Date literals, starting with @, such as @2022-01-01
    Date(String),
    /// DateTime literals with optional time and timezone parts
    /// First String is the date part, Option contains time and optional timezone
    DateTime(String, Option<(String, Option<String>)>),
    /// Time literals, starting with @T, such as @T12:00:00
    Time(String),
    /// Quantity values with a numeric value and a unit, such as 5 'mg'
    Quantity(Decimal, String),
}

/// Represents a FHIRPath expression
///
/// This enum represents the different kinds of expressions that can appear
/// in a FHIRPath expression tree. It forms the core of the abstract syntax tree (AST)
/// produced by the parser. Each variant corresponds to a different type of expression
/// in the FHIRPath language, including basic terms, operators, and function invocations.
///
/// The Expression tree is built during parsing and later evaluated by the evaluator
/// to produce a result value. The structure preserves operator precedence and
/// expression nesting as specified in the FHIRPath grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    /// A basic term (literal, invocation, etc.)
    Term(Term),

    /// A method or function invocation on an expression
    /// (e.g., `Patient.name.given.first()`)
    Invocation(Box<Expression>, Invocation),

    /// An indexer expression (e.g., `Patient.name[0]`)
    Indexer(Box<Expression>, Box<Expression>),

    /// A unary polarity expression (+ or -)
    /// (e.g., `-5` or `+value`)
    Polarity(char, Box<Expression>),

    /// A multiplicative expression (*, /, div, mod)
    /// (e.g., `value * 2` or `amount div 10`)
    Multiplicative(Box<Expression>, String, Box<Expression>),

    /// An additive expression (+ or -)
    /// (e.g., `value + 5` or `total - tax`)
    Additive(Box<Expression>, String, Box<Expression>),

    /// A type operation (is, as)
    /// (e.g., `value is Integer` or `patient as Patient`)
    Type(Box<Expression>, String, TypeSpecifier),

    /// A union operation (|)
    /// (e.g., `Patient.name | Patient.address`)
    Union(Box<Expression>, Box<Expression>),

    /// An inequality comparison (<, <=, >, >=)
    /// (e.g., `value > 5` or `date <= today()`)
    Inequality(Box<Expression>, String, Box<Expression>),

    /// An equality comparison (=, !=, ~, !~)
    /// (e.g., `name = 'John'` or `birthDate ~ @2020`)
    Equality(Box<Expression>, String, Box<Expression>),

    /// A membership test (in, contains)
    /// (e.g., `'John' in Patient.name.given` or `Patient.name contains 'John'`)
    Membership(Box<Expression>, String, Box<Expression>),

    /// A logical AND operation
    /// (e.g., `value > 5 and value < 10`)
    And(Box<Expression>, Box<Expression>),

    /// A logical OR or XOR operation
    /// (e.g., `status = 'active' or status = 'pending'`)
    Or(Box<Expression>, String, Box<Expression>),

    /// A logical IMPLIES operation
    /// (e.g., `exists() implies value > 0`)
    Implies(Box<Expression>, Box<Expression>),

    /// A lambda expression with optional identifier
    /// (e.g., `item => item.value > 10`)
    Lambda(Option<String>, Box<Expression>),
}

/// Represents a type specifier in FHIRPath
///
/// This enum is used to represent types in type operations like 'is' and 'as'.
/// It supports both simple types and namespace-qualified types as defined in the
/// FHIRPath specification.
///
/// Type specifiers are used in expressions like:
/// - `value is Integer`
/// - `patient is FHIR.Patient`
/// - `value as System.Decimal`
///
/// The parser determines whether an identifier is a simple type name or a
/// namespace-qualified type name based on the presence of a dot separator.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeSpecifier {
    /// A qualified identifier representing a type, possibly with a namespace
    ///
    /// The first String is either:
    /// - The namespace (when `Option<String>` is Some), or
    /// - The type name (when `Option<String>` is None)
    ///
    /// The `Option<String>` is:
    /// - Some(type_name) when a namespace is provided, or
    /// - None when it's a simple type without a namespace
    ///
    /// Examples:
    /// - FHIR.Patient -> QualifiedIdentifier("FHIR", Some("Patient"))
    /// - Boolean -> QualifiedIdentifier("Boolean", None)
    /// - System.Boolean -> QualifiedIdentifier("System", Some("Boolean"))
    QualifiedIdentifier(String, Option<String>),
}

/// Represents a basic term in a FHIRPath expression
///
/// A term is the most fundamental unit in a FHIRPath expression.
/// It can be a literal value, an invocation, a variable reference,
/// or a parenthesized expression. Terms are the leaves of the expression
/// tree in the abstract syntax tree (AST).
///
/// Terms can appear alone or as part of more complex expressions,
/// and they are the starting point for expression evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    /// An invocation, such as a member access, function call, or special identifier
    /// (e.g., `name`, `first()`, `$this`)
    Invocation(Invocation),

    /// A literal value like a number, string, boolean, or date
    /// (e.g., `42`, `'text'`, `true`, `@2022-01-01`)
    Literal(Literal),

    /// An external constant or environment variable reference
    /// (e.g., `%context`, `%ucum`, `%terminologies`)
    ExternalConstant(String),

    /// A parenthesized expression
    /// (e.g., `(1 + 2)`, `(Patient.name)`)
    Parenthesized(Box<Expression>),
}

/// Represents an invocation in a FHIRPath expression
///
/// An invocation represents different ways to reference or call something in FHIRPath.
/// This includes member access, function calls, and special contextual identifiers
/// like $this, $index, and $total.
///
/// Invocations are fundamental building blocks in FHIRPath expressions and
/// are used for navigation, function application, and context references.
#[derive(Debug, Clone, PartialEq)]
pub enum Invocation {
    /// A member access, referencing a property by name
    /// (e.g., `Patient.name`, `Observation.value`)
    Member(String),

    /// A function call with optional arguments
    /// (e.g., `first()`, `where(value > 5)`, `substring(2, 5)`)
    Function(String, Vec<Expression>),

    /// A reference to the current focus item ($this)
    /// Used in expressions like `$this.name` or in lambda expressions
    This,

    /// A reference to the current index ($index)
    /// Used in expressions like `$index > 5` in filtering operations
    Index,

    /// A reference to the current aggregate total ($total)
    /// Used in the aggregate() function to access the running total
    Total,
}

// Removed Unit, DateTimePrecision, PluralDateTimePrecision enums

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Null => write!(f, "{{}}"),
            Literal::Boolean(b) => write!(f, "{}", b),
            Literal::String(s) => write!(f, "'{}'", s),
            Literal::Number(d) => write!(f, "{}", d), // Use Decimal's Display
            Literal::Integer(n) => write!(f, "{}", n),
            Literal::Date(d) => write!(f, "@{}", d),
            Literal::DateTime(date, time_part) => {
                write!(f, "@{}T", date)?;
                if let Some((time, timezone)) = time_part {
                    write!(f, "{}", time)?;
                    if let Some(tz) = timezone {
                        write!(f, "{}", tz)?;
                    }
                }
                Ok(())
            }
            Literal::Time(t) => write!(f, "@T{}", t),
            Literal::Quantity(d, u) => write!(f, "{} '{}'", d, u), // Use Decimal's Display and unit string
        }
    }
}

/// Creates a parser for FHIRPath expressions
///
/// This function creates and returns a parser that can parse FHIRPath expressions
/// according to the official FHIRPath grammar specification. The parser uses the
/// chumsky parsing library to implement a recursive descent parser with proper
/// handling of operator precedence and associativity.
///
/// The parser handles all FHIRPath syntax elements including:
/// - Literals (numbers, strings, dates, times, etc.)
/// - Path navigation and member access
/// - Function invocation
/// - Mathematical operations
/// - Logical operations
/// - Comparison and equality tests
/// - Collection operators
/// - Type testing operations
///
/// # Returns
///
/// A parser that can consume a string of characters and produce an Expression
/// representing the abstract syntax tree (AST) of the parsed FHIRPath expression.
///
/// # Errors
///
/// The parser returns detailed error information when it encounters syntax errors
/// in the input, including the location and nature of the error.
pub fn parser() -> impl Parser<char, Expression, Error = Simple<char>> + Clone {
    // Parser for escape sequences within string literals
    // Handles standard escape sequences like \n, \t, \r, etc., plus Unicode
    // escape sequences in the form \uXXXX where XXXX is a 4-digit hex code.
    let esc = just('\\').ignore_then(choice((
        just('`').to('`'),        // Backtick escape
        just('\'').to('\''),      // Single quote escape
        just('\\').to('\\'),      // Backslash escape
        just('/').to('/'),        // Forward slash escape
        just('f').to('\u{000C}'), // Form feed
        just('n').to('\n'),       // Newline
        just('r').to('\r'),       // Carriage return
        just('t').to('\t'),       // Tab
        just('"').to('"'),        // Double quote escape
        // Unicode escape sequence: \uXXXX
        just('u').ignore_then(
            filter(|c: &char| c.is_ascii_hexdigit())
                .repeated()
                .exactly(4) // Require exactly 4 hex digits
                .collect::<String>()
                .validate(|digits, span, emit| {
                    match u32::from_str_radix(&digits, 16) {
                        Ok(code) => match char::from_u32(code) {
                            Some(c) => c,
                            None => {
                                emit(Simple::custom(span, "Invalid Unicode code point"));
                                ' ' // Placeholder for invalid code point
                            }
                        },
                        Err(_) => {
                            emit(Simple::custom(span, "Invalid hex digits"));
                            ' ' // Placeholder for invalid hex
                        }
                    }
                }),
        ),
    )));

    // LITERAL PARSERS

    // Parser for null/empty literals: {}
    // In FHIRPath, the empty collection is represented as {}
    let null = just('{').then(just('}')).to(Literal::Null);

    // Parser for boolean literals: true, false
    // Note: These need to be parsed before identifiers to avoid ambiguity
    let boolean = choice((
        text::keyword::<char, _, Simple<char>>("true").to(Literal::Boolean(true)),
        text::keyword::<char, _, Simple<char>>("false").to(Literal::Boolean(false)),
    ))
    .boxed();

    // Parser for string literals: 'text'
    // Handles escape sequences and allows any characters between single quotes
    let string = just('\'')
        .ignore_then(
            none_of("\\\'") // Any character except \ or '
                .or(esc.clone()) // Or an escape sequence
                .repeated()
                .collect::<String>(),
        )
        .then_ignore(just('\'')) // End with a closing quote
        .map(Literal::String) // Convert to String literal
        .boxed();

    // Parser for integer literals
    //
    // Parses sequences of digits without a decimal point into an i64 value.
    // The FHIRPath specification defines integers as 64-bit signed values.
    // This parser validates that the integer is within the valid range.
    let integer = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(1) // Require at least one digit
        .collect::<String>()
        .validate(|digits, span, emit| match i64::from_str(&digits) {
            Ok(n) => Literal::Integer(n),
            Err(_) => {
                emit(Simple::custom(span, format!("Invalid integer: {}", digits)));
                Literal::Integer(0) // Default value on error
            }
        })
        .padded(); // Allow whitespace around integers

    // Parser for decimal number literals
    //
    // Parses numbers with a decimal point into a Decimal value.
    // The FHIRPath specification uses arbitrary precision decimal values,
    // represented here using the rust_decimal crate's Decimal type.
    //
    // Format: <digits>.<digits>
    // Example: 3.14159
    let number = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(1) // Require at least one digit before the decimal
        .collect::<String>()
        .then(just('.')) // Require the decimal point
        .then(
            filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                .repeated()
                .at_least(1) // Require at least one digit after the decimal
                .collect::<String>(),
        )
        .validate(|((i, _), d), span, emit| {
            let num_str = format!("{}.{}", i, d);
            match Decimal::from_str(&num_str) {
                Ok(decimal) => Literal::Number(decimal),
                Err(_) => {
                    emit(Simple::custom(span, format!("Invalid number: {}", num_str)));
                    Literal::Number(dec!(0)) // Default value on error
                }
            }
        })
        .padded(); // Allow whitespace around numbers

    // Parser for time format components
    //
    // Handles the FHIRPath time format: HH(:mm(:ss(.sss)?)?)?
    // This can be as simple as just hours (HH) or include minutes,
    // seconds, and milliseconds with the appropriate separators.
    //
    // Examples:
    // - 12 (just hours)
    // - 14:30 (hours and minutes)
    // - 09:45:30 (hours, minutes, seconds)
    // - 23:59:59.999 (hours, minutes, seconds, and milliseconds)
    let time_format = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(2) // Hours: exactly 2 digits
        .at_most(2)
        .collect::<String>()
        .then(
            just(':') // Optional minutes part
                .ignore_then(
                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                        .repeated()
                        .at_least(2) // Minutes: exactly 2 digits
                        .at_most(2)
                        .collect::<String>(),
                )
                .then(
                    just(':') // Optional seconds part
                        .ignore_then(
                            filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                                .repeated()
                                .at_least(2) // Seconds: exactly 2 digits
                                .at_most(2)
                                .collect::<String>(),
                        )
                        .then(
                            just('.') // Optional milliseconds part
                                .ignore_then(
                                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                                        .repeated()
                                        .at_least(1) // Milliseconds: 1-3 digits
                                        .at_most(3)
                                        .collect::<String>(),
                                )
                                .or_not(),
                        )
                        .or_not(),
                )
                .or_not(),
        )
        .map(|(hours, rest_opt)| {
            // Combine all the parts into a single time string
            let mut result = hours;
            if let Some((minutes, seconds_part)) = rest_opt {
                result.push(':');
                result.push_str(&minutes);

                if let Some((seconds, milliseconds)) = seconds_part {
                    result.push(':');
                    result.push_str(&seconds);

                    // milliseconds is an Option<String>
                    if let Some(ms) = milliseconds {
                        result.push('.');
                        result.push_str(&ms);
                    }
                }
            }
            result
        });

    // Parser for timezone format
    //
    // Handles the two timezone formats defined in FHIRPath:
    // - 'Z' for UTC/Zulu time
    // - (+|-)HH:mm for timezone offset (e.g., +01:00, -05:30)
    //
    // This parser validates the format and produces a string
    // representation of the timezone.
    let timezone_format = just('Z')
        .to("Z".to_string()) // UTC/Zulu time
        .or(one_of("+-") // Or timezone offset
            .map(|c: char| c.to_string()) // Get sign as string
            .then(
                filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                    .repeated()
                    .at_most(2) // Hours: exactly 2 digits
                    .at_least(2)
                    .collect::<String>(),
            )
            .then(just(':')) // Colon separator
            .then(
                filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                    .repeated()
                    .at_most(2) // Minutes: exactly 2 digits
                    .at_least(2)
                    .collect::<String>(),
            )
            .map(|(((sign, hour), _), min)| format!("{}{}:{}", sign, hour, min)));

    // Parser for date format
    //
    // Handles the FHIRPath date format: YYYY(-MM(-DD)?)?
    // This parser supports multiple date precisions:
    // - Year only: YYYY (e.g., 2022)
    // - Year and month: YYYY-MM (e.g., 2022-01)
    // - Full date: YYYY-MM-DD (e.g., 2022-01-15)
    //
    // The parser validates the format and produces a string representation
    // of the date for use in Date literals.
    //
    // Examples:
    //
    // - 1972 (year only)
    // - 2015-12 (year and month)
    // - 1972-12-14 (full date)
    let date_format_str = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .exactly(4) // Year: exactly 4 digits
        .collect::<String>()
        .then(
            just('-') // Optional month part
                .ignore_then(
                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                        .repeated()
                        .exactly(2) // Month: exactly 2 digits
                        .collect::<String>()
                        .then(
                            just('-') // Optional day part
                                .ignore_then(
                                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                                        .repeated()
                                        .exactly(2) // Day: exactly 2 digits
                                        .collect::<String>(),
                                )
                                .or_not(),
                        ),
                )
                .or_not(),
        )
        .map(|(year, month_part)| {
            // Combine all the parts into a single date string
            let mut date_str = year;

            // month_part is Option<(month_str, Option<day_str>)>
            if let Some((month_str, day_part)) = month_part {
                date_str.push('-');
                date_str.push_str(&month_str);

                // day_part is Option<day_str>
                if let Some(day_str) = day_part {
                    date_str.push('-');
                    date_str.push_str(&day_str);
                }
            }

            date_str // Returns String
        })
        .boxed();

    // Parser for unit values in quantity literals
    //
    // Units in FHIRPath can be specified either as predefined time unit keywords
    // or as arbitrary string literals enclosed in single quotes.
    //
    // This parser handles both forms:
    // - Time unit keywords (year, month, day, hour, minute, second, etc.)
    // - String literal units ('mg', 'kg', 'cm', etc.)
    //
    // The parser returns the unit as a String regardless of which form was used.

    // Parser for time unit keywords
    // These are the predefined time unit keywords in FHIRPath
    let unit_keyword = choice((
        // Singular forms
        text::keyword("year").to("year".to_string()),
        text::keyword("month").to("month".to_string()),
        text::keyword("week").to("week".to_string()),
        text::keyword("day").to("day".to_string()),
        text::keyword("hour").to("hour".to_string()),
        text::keyword("minute").to("minute".to_string()),
        text::keyword("second").to("second".to_string()),
        text::keyword("millisecond").to("millisecond".to_string()),
        // Plural forms
        text::keyword("years").to("years".to_string()),
        text::keyword("months").to("months".to_string()),
        text::keyword("weeks").to("weeks".to_string()),
        text::keyword("days").to("days".to_string()),
        text::keyword("hours").to("hours".to_string()),
        text::keyword("minutes").to("minutes".to_string()),
        text::keyword("seconds").to("seconds".to_string()),
        text::keyword("milliseconds").to("milliseconds".to_string()),
    ));

    // Parser for string literal units
    // These are arbitrary units enclosed in single quotes
    let unit_string_literal = just('\'')
        .ignore_then(
            none_of("\\\'") // Any character except \ or '
                .or(esc.clone()) // Or an escape sequence
                .repeated()
                .collect::<String>(),
        )
        .then_ignore(just('\''));

    // Combined parser for all unit forms
    let unit = choice((
        unit_keyword,        // Time unit keywords
        unit_string_literal, // String literal units
    ))
    .boxed() // Box for recursive definitions
    .padded(); // Allow whitespace around units

    // Define integer/number parsers specifically for quantity, without consuming trailing whitespace.
    let integer_for_quantity = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(1)
        .collect::<String>()
        .validate(|digits, span, emit| match i64::from_str(&digits) {
            Ok(n) => n, // Return the i64 directly
            Err(_) => {
                emit(Simple::custom(span, format!("Invalid integer: {}", digits)));
                0
            }
        });

    let number_for_quantity = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(1)
        .collect::<String>()
        .then(just('.'))
        .then(
            filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                .repeated()
                .at_least(1)
                .collect::<String>(),
        )
        .validate(|((i, _), d), span, emit| {
            let num_str = format!("{}.{}", i, d);
            match Decimal::from_str(&num_str) {
                Ok(decimal) => decimal, // Return the Decimal directly
                Err(_) => {
                    emit(Simple::custom(span, format!("Invalid number: {}", num_str)));
                    dec!(0)
                }
            }
        });

    // Quantity parser: (integer_for_quantity | number_for_quantity) + required whitespace + unit
    // This parser consumes the whole quantity structure.
    let quantity = choice((
        // Try integer quantity first
        integer_for_quantity
            .then_ignore(text::whitespace().at_least(1)) // Require whitespace
            .then(unit.clone()) // Parse the unit string
            .map(|(i, u_str)| Literal::Quantity(Decimal::from(i), u_str)), // Create Literal::Quantity with Decimal and String unit
        // Then try decimal quantity
        number_for_quantity
            .then_ignore(text::whitespace().at_least(1)) // Require whitespace
            .then(unit.clone()) // Parse the unit string
            .map(|(d, u_str)| Literal::Quantity(d, u_str)), // Create Literal::Quantity with Decimal and String unit
    ));

    // Removed unused emit_error helper function

    // Parser for DateTime: @Date T Time [Timezone]
    let datetime_literal = just('@')
        .ignore_then(date_format_str.clone())
        .then_ignore(just('T'))
        .then(time_format.clone())
        .then(timezone_format.clone().or_not())
        .map(|((date_str, time_str), tz_opt)| {
            Literal::DateTime(date_str, Some((time_str, tz_opt)))
        });

    // Parser for Partial DateTime: @Date T
    let partial_datetime_literal = just('@')
        .ignore_then(date_format_str.clone())
        .then_ignore(just('T'))
        .map(|date_str| {
            Literal::DateTime(date_str, None) // No time numbers, no timezone
        });

    // Parser for Time: @ T Time (strictly no timezone)
    // Uses try_map to fail parsing if a timezone is present.
    let time_literal = just('@')
        .ignore_then(
            just('T')
                .ignore_then(time_format.clone())
                .then(timezone_format.clone().or_not()), // Parse time and optional timezone
        )
        .try_map(|(time_str, tz_opt), span| {
            // Validate that timezone is not present
            if tz_opt.is_some() {
                Err(Simple::custom(
                    span,
                    "Time literal cannot have a timezone offset",
                ))
            } else {
                Ok(Literal::Time(time_str))
            }
        });

    // Parser for Date: @ Date
    let date_literal = just('@')
        .ignore_then(date_format_str.clone())
        .map(Literal::Date);

    // Order matters: try quantity before plain number/integer.
    // Specific date/time formats should be tried before more general ones if there's ambiguity,
    // though the new structure aims to make them distinct.
    let literal = choice((
        null,
        boolean,
        string,
        quantity,                          // Try quantity first
        number,                            // Then number (requires '.')
        integer,                           // Then integer
        datetime_literal.padded(),         // @Date T Time [TZ]
        partial_datetime_literal.padded(), // @Date T
        time_literal.padded(),             // @ T Time (will fail if TZ present)
        date_literal.padded(),             // @Date
    ))
    .map(Term::Literal);

    // IDENTIFIER: ([A-Za-z] | '_')([A-Za-z0-9] | '_')*
    let standard_identifier = filter(|c: &char| c.is_ascii_alphabetic() || *c == '_')
        .then(filter(|c: &char| c.is_ascii_alphanumeric() || *c == '_').repeated())
        .map(|(first, rest)| {
            let mut s = first.to_string();
            s.extend(rest);
            s
        })
        .padded();

    // DELIMITEDIDENTIFIER: '`' (ESC | .)*? '`'
    let delimited_identifier = just('`')
        .ignore_then(none_of("`").or(esc.clone()).repeated().collect::<String>())
        .then_ignore(just('`'))
        .padded();

    // Combined identifier parser - allow true/false as identifiers
    // Also allow keywords used in specific contexts (like 'as', 'is') to be parsed as identifiers
    // when they appear where an identifier is expected (e.g., in function calls or member access).
    // The context of the grammar will differentiate their use.
    let identifier = choice((
        standard_identifier.clone(),
        delimited_identifier,
        // Allow keywords to be parsed as identifiers if they appear in identifier positions
        text::keyword("as").to(String::from("as")),
        text::keyword("contains").to(String::from("contains")),
        text::keyword("in").to(String::from("in")),
        text::keyword("is").to(String::from("is")),
        text::keyword("true").to(String::from("true")), // Allow 'true' as identifier
        text::keyword("false").to(String::from("false")), // Allow 'false' as identifier
                                                        // Add other keywords if they can appear as identifiers in some contexts
    ));

    // Qualified identifier (for type specifiers)
    // Handles all these patterns:
    // - Single identifier: Boolean, Patient, etc.
    // - Namespace.Type: System.Boolean, FHIR.Patient
    // - Backtick quoted: `System`.`Boolean`, FHIR.`Patient`
    let qualified_identifier = {
        // First try to handle explicit namespace.type pattern
        let explicit_namespace_type = identifier
            .clone()
            .then(just('.').ignore_then(identifier.clone()))
            .map(|(namespace, type_name)| {
                // Clean both parts (removing backticks if present)
                let clean_ns = clean_backtick_identifier(&namespace);
                let clean_type = clean_backtick_identifier(&type_name);
                TypeSpecifier::QualifiedIdentifier(clean_ns, Some(clean_type))
            });

        // Then handle standalone identifiers (which might themselves contain dots)
        let standalone_type = identifier.clone().map(|id| {
            let clean_id = clean_backtick_identifier(&id);

            // Check if this identifier already contains dots (like "System.Boolean")
            if clean_id.contains('.') {
                // This might be a pre-qualified identifier typed directly
                // Split at the last dot to get namespace and type
                if let Some(last_dot_pos) = clean_id.rfind('.') {
                    let namespace = clean_id[..last_dot_pos].to_string();
                    let type_name = clean_id[last_dot_pos + 1..].to_string();
                    TypeSpecifier::QualifiedIdentifier(namespace, Some(type_name))
                } else {
                    // Shouldn't happen if contains('.') returned true, but just in case
                    TypeSpecifier::QualifiedIdentifier(clean_id, None)
                }
            } else {
                // Simple unqualified type name
                TypeSpecifier::QualifiedIdentifier(clean_id, None)
            }
        });

        // Try explicit namespace.type first, then fallback to standalone identifier
        choice((explicit_namespace_type.boxed(), standalone_type.boxed()))
            .padded()
            .boxed()
    };

    // Helper function to remove backticks from identifiers if present
    fn clean_backtick_identifier(id: &str) -> String {
        if id.starts_with('`') && id.ends_with('`') && id.len() >= 3 {
            id[1..id.len() - 1].to_string()
        } else {
            id.to_string()
        }
    }

    // Create a separate string parser for external constants
    let string_for_external = just('\'')
        .ignore_then(
            none_of("\'\\")
                .or(esc.clone())
                .repeated()
                .collect::<String>(),
        )
        .then_ignore(just('\''))
        .padded();

    // External constants
    let external_constant = just('%')
        .ignore_then(choice((identifier.clone(), string_for_external)))
        .map(Term::ExternalConstant)
        .padded();

    // Recursive parser definition that directly mirrors the grammar structure
    recursive(|expr| {
        // Atom: the most basic elements like literals, identifiers, parenthesized expressions.
        let atom = choice((
            // Box each branch individually to ensure type uniformity for choice
            literal.clone().map(Expression::Term).boxed(), // Map literal Term to Expression here
            external_constant.clone().map(Expression::Term).boxed(),
            // Function call: identifier(...) - Try this *before* simple identifier
            identifier
                .clone()
                .then(
                    expr.clone()
                        .separated_by(just(',').padded())
                        .allow_trailing()
                        .collect::<Vec<_>>()
                        // Ensure parentheses are padded to handle potential whitespace
                        .delimited_by(just('(').padded(), just(')').padded()),
                )
                // Directly create the Expression::Term(Term::Invocation(...)) structure
                .map(|(name, params)| {
                    Expression::Term(Term::Invocation(Invocation::Function(name, params)))
                })
                .boxed(),
            // Simple identifier, $this, $index, $total (parsed if not a function call)
            choice((
                identifier.clone().map(Invocation::Member),
                just("$this").to(Invocation::This),
                just("$index").to(Invocation::Index),
                just("$total").to(Invocation::Total),
            ))
            .map(Term::Invocation) // Map these simple invocations to Term
            .map(Expression::Term) // Map Term to Expression
            .boxed(),
            // Parenthesized expression
            expr.clone()
                .delimited_by(just('(').padded(), just(')').padded())
                // Parenthesized expression directly yields an Expression
                .boxed(),
        ))
        .padded();

        // Postfix operators: . (member/function invocation) and [] (indexer)
        let postfix_op = choice((
            // Member/Function Invocation: '.' followed by identifier, optionally followed by args (...)
            just('.')
                .ignore_then(
                    identifier.clone().then(
                        // Optionally parse arguments
                        expr.clone()
                            .separated_by(just(',').padded())
                            .allow_trailing()
                            .collect::<Vec<_>>()
                            .delimited_by(just('(').padded(), just(')').padded())
                            .or_not(), // Make arguments optional
                    ),
                )
                .map(|(name, params_opt)| {
                    // Create the correct Invocation based on whether params were found
                    let invocation = match params_opt {
                        Some(params) => Invocation::Function(name, params),
                        None => Invocation::Member(name),
                    };
                    // Return the closure
                    Box::new(move |left: Expression| {
                        Expression::Invocation(Box::new(left), invocation.clone())
                    }) as Box<dyn Fn(Expression) -> Expression>
                }),
            // Indexer
            expr.clone()
                .delimited_by(just('[').padded(), just(']').padded())
                .map(|idx| {
                    Box::new(move |left: Expression| {
                        Expression::Indexer(Box::new(left), Box::new(idx.clone()))
                    }) as Box<dyn Fn(Expression) -> Expression>
                }),
        ))
        .boxed(); // Box the choice result

        let atom_with_postfix = atom
            .clone()
            .then(postfix_op.repeated())
            .foldl(|left, op_fn| op_fn(left));

        // Prefix operators (Polarity)
        let prefix_op = choice((
            just::<_, _, Simple<char>>('+').to('+'),
            just::<_, _, Simple<char>>('-').to('-'),
        ))
        .padded();

        let term_with_polarity = prefix_op
            .repeated()
            .then(atom_with_postfix)
            .foldr(|op, right| Expression::Polarity(op, Box::new(right)));

        // Infix operators with precedence levels (from high to low)

        // Level 1: Multiplicative (*, /, div, mod) - Left associative
        let op_mul = choice((
            just('*').to("*"),
            just('/').to("/"),
            text::keyword("div").to("div"),
            text::keyword("mod").to("mod"),
        ))
        .padded();
        let multiplicative = term_with_polarity
            .clone()
            .then(op_mul.then(term_with_polarity).repeated())
            .foldl(|left, (op_str, right)| {
                Expression::Multiplicative(Box::new(left), op_str.to_string(), Box::new(right))
            });

        // Level 2: Additive (+, -, &) - Left associative
        let op_add = choice((just('+').to("+"), just('-').to("-"), just('&').to("&"))).padded();
        let additive = multiplicative
            .clone()
            .then(op_add.then(multiplicative).repeated())
            .foldl(|left, (op_str, right)| {
                Expression::Additive(Box::new(left), op_str.to_string(), Box::new(right))
            });

        // Level 3: Union (|) - Left associative (though spec doesn't strictly define associativity here)
        let op_union = just('|').padded();
        let union = additive
            .clone()
            .then(op_union.then(additive).repeated())
            .foldl(|left, (_, right)| Expression::Union(Box::new(left), Box::new(right)));

        // Level 4: Type (is, as) - Left associative
        let op_type = choice((text::keyword("is").to("is"), text::keyword("as").to("as"))).padded();
        let type_expr = union
            .clone()
            .then(op_type.then(qualified_identifier.clone()).repeated()) // Type specifier follows 'is'/'as'
            .foldl(|left, (op_str, type_spec)| {
                Expression::Type(Box::new(left), op_str.to_string(), type_spec)
            });

        // Level 5: Inequality (<, <=, >, >=) - Left associative
        let op_ineq = choice((
            just("<=").to("<="),
            just("<").to("<"),
            just(">=").to(">="),
            just(">").to(">"),
        ))
        .padded();
        let inequality = type_expr
            .clone()
            .then(op_ineq.then(type_expr).repeated())
            .foldl(|left, (op_str, right)| {
                Expression::Inequality(Box::new(left), op_str.to_string(), Box::new(right))
            });

        // Level 6: Equality (=, ~, !=, !~) - Left associative
        let op_eq = choice((
            just("=").to("="),
            just("~").to("~"),
            just("!=").to("!="),
            just("!~").to("!~"),
        ))
        .padded();
        let equality = inequality
            .clone()
            .then(op_eq.then(inequality).repeated())
            .foldl(|left, (op_str, right)| {
                Expression::Equality(Box::new(left), op_str.to_string(), Box::new(right))
            });

        // Level 7: Membership (in, contains) - Left associative
        let op_mem = choice((
            text::keyword("in").to("in"),
            text::keyword("contains").to("contains"),
        ))
        .padded();
        let membership = equality
            .clone()
            .then(op_mem.then(equality).repeated())
            .foldl(|left, (op_str, right)| {
                Expression::Membership(Box::new(left), op_str.to_string(), Box::new(right))
            });

        // Level 8: Logical AND (and) - Left associative
        let op_and = text::keyword("and").padded();
        let logical_and = membership
            .clone()
            .then(op_and.then(membership).repeated())
            .foldl(|left, (_, right)| Expression::And(Box::new(left), Box::new(right)));

        // Level 9: Logical OR/XOR (or, xor) - Left associative
        let op_or = choice((text::keyword("or").to("or"), text::keyword("xor").to("xor"))).padded();
        let logical_or = logical_and
            .clone()
            .then(op_or.then(logical_and).repeated())
            .foldl(|left, (op_str, right)| {
                Expression::Or(Box::new(left), op_str.to_string(), Box::new(right))
            });

        // Level 10: Implies (implies) - Right associative (or handle as non-assoc if simpler)
        // Let's treat it as left-associative for simplicity, though the spec implies right-to-left evaluation.
        // A proper right-associative fold might be needed if complex implies chains are common.
        let op_implies = text::keyword("implies").padded();
        let implies = logical_or
            .clone()
            .then(op_implies.then(logical_or).repeated())
            .foldl(|left, (_, right)| Expression::Implies(Box::new(left), Box::new(right)));

        // The final expression parser is the one with the lowest precedence
        implies // This is now the return value of the closure
    }) // Close the recursive closure here
    .then_ignore(end()) // Ensure the entire input is consumed after the expression
}
