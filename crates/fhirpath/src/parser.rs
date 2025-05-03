// Removed direct import of attempt
use chumsky::error::Simple;
use chumsky::prelude::*;
use chumsky::Parser;
use rust_decimal::Decimal;
use rust_decimal_macros::dec; // For potential default values if needed
use std::fmt;
// Removed duplicate fmt import
use std::str::FromStr;

// Removed PostfixOpFn type alias

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    Boolean(bool),
    String(String),
    Number(Decimal), // Represents numbers with a decimal point
    Integer(i64),    // Represents numbers without a decimal point
    Date(String),
    DateTime(String, Option<(String, Option<String>)>),
    Time(String),
    Quantity(Decimal, Option<Unit>), // Changed from f64 to Decimal
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Term(Term),
    Invocation(Box<Expression>, Invocation),
    Indexer(Box<Expression>, Box<Expression>),
    Polarity(char, Box<Expression>),
    Multiplicative(Box<Expression>, String, Box<Expression>),
    Additive(Box<Expression>, String, Box<Expression>),
    Type(Box<Expression>, String, TypeSpecifier),
    Union(Box<Expression>, Box<Expression>),
    Inequality(Box<Expression>, String, Box<Expression>),
    Equality(Box<Expression>, String, Box<Expression>),
    Membership(Box<Expression>, String, Box<Expression>),
    And(Box<Expression>, Box<Expression>),
    Or(Box<Expression>, String, Box<Expression>),
    Implies(Box<Expression>, Box<Expression>),
    Lambda(Option<String>, Box<Expression>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeSpecifier {
    QualifiedIdentifier(String, Option<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    Invocation(Invocation),
    Literal(Literal),
    ExternalConstant(String),
    Parenthesized(Box<Expression>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Invocation {
    Member(String),
    Function(String, Vec<Expression>),
    This,
    Index,
    Total,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Unit {
    DateTimePrecision(DateTimePrecision),
    PluralDateTimePrecision(PluralDateTimePrecision),
    UCUM(String),
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unit::DateTimePrecision(p) => write!(f, "{:?}", p).map(|_| ()),
            Unit::PluralDateTimePrecision(p) => write!(f, "{:?}", p).map(|_| ()),
            Unit::UCUM(s) => write!(f, "{}", s),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DateTimePrecision {
    Year,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
    Millisecond,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PluralDateTimePrecision {
    Years,
    Months,
    Weeks,
    Days,
    Hours,
    Minutes,
    Seconds,
    Milliseconds,
}

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
            Literal::Quantity(d, Some(u)) => write!(f, "{} '{}'", d, u), // Use Decimal's Display
            Literal::Quantity(d, None) => write!(f, "{}", d), // Use Decimal's Display
        }
    }
}

pub fn parser() -> impl Parser<char, Expression, Error = Simple<char>> + Clone {
    // Define escape sequence parser
    let esc = just('\\').ignore_then(choice((
        just('`').to('`'),
        just('\'').to('\''),
        just('\\').to('\\'),
        just('/').to('/'),
        just('f').to('\u{000C}'), // form feed
        just('n').to('\n'),
        just('r').to('\r'),
        just('t').to('\t'),
        just('"').to('"'), // Add support for double quote escape
        just('u').ignore_then(
            filter(|c: &char| c.is_ascii_hexdigit())
                .repeated()
                .exactly(4)
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

    // Literals
    let null = just('{').then(just('}')).to(Literal::Null);

    // Make sure boolean literals are parsed before identifiers
    let boolean = choice((
        text::keyword::<char, _, Simple<char>>("true").to(Literal::Boolean(true)),
        text::keyword::<char, _, Simple<char>>("false").to(Literal::Boolean(false)),
    ))
    .boxed();

    let string = just('\'')
        .ignore_then(
            none_of("\\\'")
                .or(esc.clone())
                .repeated()
                .collect::<String>(),
        )
        .then_ignore(just('\''))
        .map(Literal::String)
        .boxed();

    // Integer parser: matches sequences of digits without a decimal point.
    let integer = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(1)
        .collect::<String>()
        .validate(|digits, span, emit| match i64::from_str(&digits) {
            Ok(n) => Literal::Integer(n),
            Err(_) => {
                emit(Simple::custom(span, format!("Invalid integer: {}", digits)));
                Literal::Integer(0) // Default value on error
            }
        })
        .padded(); // Allow whitespace around integers

    // Number parser: matches sequences of digits WITH a decimal point.
    let number = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(1)
        .collect::<String>()
        .then(just('.')) // Require the decimal point
        .then(
            filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                .repeated()
                .at_least(1) // Require digits after the decimal point
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

    // Time format: HH(:mm(:ss(.sss)?)?)?
    let time_format = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(2)
        .at_most(2)
        .collect::<String>()
        .then(
            just(':')
                .ignore_then(
                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                        .repeated()
                        .at_least(2)
                        .at_most(2)
                        .collect::<String>(),
                )
                .then(
                    just(':')
                        .ignore_then(
                            filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                                .repeated()
                                .at_least(2)
                                .at_most(2)
                                .collect::<String>(),
                        )
                        .then(
                            just('.')
                                .ignore_then(
                                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                                        .repeated()
                                        .at_least(1)
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

    // Timezone format: Z | (+|-)HH:mm
    let timezone_format = just('Z').to("Z".to_string()).or(one_of("+-")
        .map(|c: char| c.to_string())
        .then(
            filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                .repeated()
                .at_most(2)
                .at_least(2)
                .collect::<String>(),
        )
        .then(just(':'))
        .then(
            filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                .repeated()
                .at_most(2)
                .at_least(2)
                .collect::<String>(),
        )
        .map(|(((sign, hour), _), min)| format!("{}{}:{}", sign, hour, min)));

    // Date format: YYYY(-MM(-DD)?)?
    // This handles all valid formats: 1972, 2015, 1972-12, 1972-12-14
    let date_format = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .exactly(4)
        .collect::<String>()
        .then(
            just('-')
                .ignore_then(
                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                        .repeated()
                        .exactly(2)
                        .collect::<String>()
                        .then(
                            just('-')
                                .ignore_then(
                                    filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
                                        .repeated()
                                        .exactly(2)
                                        .collect::<String>(),
                                )
                                .or_not(),
                        ),
                )
                .or_not(),
        )
        .map(|(year, month_part)| {
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

            Literal::Date(date_str)
        })
        .boxed();

    // Date time precision units
    let date_time_precision = choice((
        text::keyword("year").to(DateTimePrecision::Year),
        text::keyword("month").to(DateTimePrecision::Month),
        text::keyword("week").to(DateTimePrecision::Week),
        text::keyword("day").to(DateTimePrecision::Day),
        text::keyword("hour").to(DateTimePrecision::Hour),
        text::keyword("minute").to(DateTimePrecision::Minute),
        text::keyword("second").to(DateTimePrecision::Second),
        text::keyword("millisecond").to(DateTimePrecision::Millisecond),
    ));

    // Plural date time precision units
    let plural_date_time_precision = choice((
        text::keyword("years").to(PluralDateTimePrecision::Years),
        text::keyword("months").to(PluralDateTimePrecision::Months),
        text::keyword("weeks").to(PluralDateTimePrecision::Weeks),
        text::keyword("days").to(PluralDateTimePrecision::Days),
        text::keyword("hours").to(PluralDateTimePrecision::Hours),
        text::keyword("minutes").to(PluralDateTimePrecision::Minutes),
        text::keyword("seconds").to(PluralDateTimePrecision::Seconds),
        text::keyword("milliseconds").to(PluralDateTimePrecision::Milliseconds),
    ));

    // Unit parser - can be a date time precision, plural date time precision, or a string (UCUM syntax)
    let unit = choice((
        date_time_precision.map(Unit::DateTimePrecision),
        plural_date_time_precision.map(Unit::PluralDateTimePrecision),
        string.clone().map(|s| {
            if let Literal::String(str_val) = s {
                Unit::UCUM(str_val)
            } else {
                // This shouldn't happen due to the parser structure
                Unit::UCUM("".to_string())
            }
        }),
    ))
    .padded(); // Unit parser itself can be padded

    // Define integer/number parsers specifically for quantity, without consuming trailing whitespace.
    let integer_for_quantity = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated().at_least(1).collect::<String>()
        .validate(|digits, span, emit| match i64::from_str(&digits) {
            Ok(n) => n, // Return the i64 directly
            Err(_) => { emit(Simple::custom(span, format!("Invalid integer: {}", digits))); 0 }
        });

    let number_for_quantity = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated().at_least(1).collect::<String>()
        .then(just('.'))
        .then(filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit()).repeated().at_least(1).collect::<String>())
        .validate(|((i, _), d), span, emit| {
            let num_str = format!("{}.{}", i, d);
            match Decimal::from_str(&num_str) {
                Ok(decimal) => decimal, // Return the Decimal directly
                Err(_) => { emit(Simple::custom(span, format!("Invalid number: {}", num_str))); dec!(0) }
            }
        });

    // Quantity parser: (integer_for_quantity | number_for_quantity) + required whitespace + unit
    // This parser consumes the whole quantity structure.
    let quantity = choice((
            // Try integer quantity first
            integer_for_quantity
                .then_ignore(text::whitespace().at_least(1))
                .then(unit.clone())
                .map(|(i, u)| Literal::Quantity(Decimal::from(i), Some(u))),
            // Then try decimal quantity
            number_for_quantity
                .then_ignore(text::whitespace().at_least(1))
                .then(unit.clone())
                .map(|(d, u)| Literal::Quantity(d, Some(u))),
        ));
        // Note: No .map() or match needed here as the inner maps handle Literal creation.
        // The outer choice directly produces Literal::Quantity or fails.


    // Removed unused emit_error helper function

    let date_datetime_time = just('@')
        .ignore_then(date_format.clone().or_not())
        .then(
            just('T')
                .ignore_then(
                    time_format
                        .clone()
                        .then(timezone_format.clone().or_not())
                        .or_not(),
                )
                .or_not(),
        )
        .map(|(date_part, time_part)| {
            match (date_part, time_part) {
                // @2022-01-01T12:30 or @2022-01-01T12:30Z or @2022-01-01T12:30+01:00
                (Some(Literal::Date(date_str)), Some(Some((time_str, timezone)))) => {
                    Literal::DateTime(date_str, Some((time_str, timezone)))
                }
                // @2022-01-01T
                (Some(Literal::Date(date_str)), Some(None)) => Literal::DateTime(date_str, None),
                // @2022-01-01
                (Some(Literal::Date(date_str)), None) => Literal::Date(date_str),
                // @T12:30 or @T12:30Z or @T12:30+01:00
                (None, Some(Some((time_str, timezone)))) => {
                    // Combine time string with timezone if present
                    if let Some(tz) = &timezone {
                        Literal::Time(format!("{}{}", time_str, tz))
                    } else {
                        Literal::Time(time_str)
                    }
                }
                // @T... (handled above)
                // (None, Some(Some((time_str, None)))) => Literal::Time(time_str), // This pattern is unreachable
                // Invalid combinations or parsing errors
                _ => {
                    // This case indicates an unexpected parsing result.
                        // Log or handle this error appropriately.
                        // Returning Null might mask issues. Consider a dedicated Error literal or panic.
                        eprintln!("Warning: Unexpected combination in date/time parsing.");
                        Literal::Null // Or handle as an error
                    }
                }
            })
            .padded();

    // Order matters: try quantity before plain number/integer
    let literal = choice((
        null,
        boolean,
        string,
        quantity, // Try quantity first
        number,   // Then number (requires '.')
        integer,  // Then integer
        date_datetime_time,
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
    let identifier = choice((
        standard_identifier.clone(),
        delimited_identifier,
        text::keyword("as").to(String::from("as")),
        text::keyword("contains").to(String::from("contains")),
        text::keyword("in").to(String::from("in")),
        text::keyword("is").to(String::from("is")),
    ));

    // Qualified identifier (for type specifiers)
    let qualified_identifier = identifier
        .clone()
        .then(just('.').ignore_then(identifier.clone()).or_not())
        .map(|(namespace, name)| TypeSpecifier::QualifiedIdentifier(namespace, name))
        .padded()
        .boxed(); // Box the parser to make it easier to clone

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
            identifier.clone()
                .then(
                    expr.clone()
                        .separated_by(just(',').padded())
                        .allow_trailing()
                        .collect::<Vec<_>>()
                        // Ensure parentheses are padded to handle potential whitespace
                        .delimited_by(just('(').padded(), just(')').padded())
                )
                 // Directly create the Expression::Term(Term::Invocation(...)) structure
                .map(|(name, params)| Expression::Term(Term::Invocation(Invocation::Function(name, params))))
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
            // Member/Function Invocation: '.' followed by an identifier or function call structure
            just('.').ignore_then(
                choice((
                    // Attempt to parse identifier followed by parentheses FIRST
                    // Qualify attempt with chumsky::combinator
                    chumsky::combinator::attempt( // Use attempt to allow backtracking if parentheses are not found
                        identifier.clone()
                            .then(
                                expr.clone()
                                    .separated_by(just(',').padded())
                                .allow_trailing()
                                .collect::<Vec<_>>()
                                // Add padding to parentheses here
                                    .delimited_by(just('(').padded(), just(')').padded())
                            )
                            .map(|(name, params)| Invocation::Function(name, params))
                    ),
                    // If the above fails (no parentheses), parse as simple member access
                    identifier.clone().map(Invocation::Member),
                ))
            )
            .map(|inv| {
                // Closure now inferred, boxing happens below
                Box::new(move |left: Expression| Expression::Invocation(Box::new(left), inv.clone()))
                    as Box<dyn Fn(Expression) -> Expression> // Cast to dyn Fn trait object
            })
            .boxed(), // Box the parser returning the closure
            // Indexer
            expr.clone().delimited_by(just('[').padded(), just(']').padded())
                .map(|idx| {
                     // Closure now inferred, boxing happens below
                    Box::new(move |left: Expression| Expression::Indexer(Box::new(left), Box::new(idx.clone())))
                        as Box<dyn Fn(Expression) -> Expression> // Cast to dyn Fn trait object
                })
                .boxed(), // Box the parser returning the closure
        )).boxed(); // Box the result of the choice combinator

        let atom_with_postfix = atom.clone()
            // Now call repeated on the boxed Choice parser
            .then(postfix_op.repeated())
            .foldl(|left, op_fn| op_fn(left));

        // Prefix operators (Polarity)
        let prefix_op = choice((
            just::<_, _, Simple<char>>('+').to('+'),
            just::<_, _, Simple<char>>('-').to('-'),
        )).padded();

        let term_with_polarity = prefix_op.repeated()
            .then(atom_with_postfix)
            .foldr(|op, right| Expression::Polarity(op, Box::new(right)));

        // Infix operators with precedence levels (from high to low)

        // Level 1: Multiplicative (*, /, div, mod) - Left associative
        let op_mul = choice((
            just('*').to("*"),
            just('/').to("/"),
            text::keyword("div").to("div"),
            text::keyword("mod").to("mod"),
        )).padded();
        let multiplicative = term_with_polarity.clone()
            .then(op_mul.then(term_with_polarity).repeated())
            .foldl(|left, (op_str, right)| Expression::Multiplicative(Box::new(left), op_str.to_string(), Box::new(right)));

        // Level 2: Additive (+, -, &) - Left associative
        let op_add = choice((
            just('+').to("+"),
            just('-').to("-"),
            just('&').to("&"),
        )).padded();
        let additive = multiplicative.clone()
            .then(op_add.then(multiplicative).repeated())
            .foldl(|left, (op_str, right)| Expression::Additive(Box::new(left), op_str.to_string(), Box::new(right)));

        // Level 3: Union (|) - Left associative (though spec doesn't strictly define associativity here)
        let op_union = just('|').padded();
        let union = additive.clone()
            .then(op_union.then(additive).repeated())
            .foldl(|left, (_, right)| Expression::Union(Box::new(left), Box::new(right)));

        // Level 4: Type (is, as) - Left associative
        let op_type = choice((
            text::keyword("is").to("is"),
            text::keyword("as").to("as"),
        )).padded();
        let type_expr = union.clone()
            .then(op_type.then(qualified_identifier.clone()).repeated()) // Type specifier follows 'is'/'as'
            .foldl(|left, (op_str, type_spec)| Expression::Type(Box::new(left), op_str.to_string(), type_spec));

        // Level 5: Inequality (<, <=, >, >=) - Left associative
        let op_ineq = choice((
            just("<=").to("<="),
            just("<").to("<"),
            just(">=").to(">="),
            just(">").to(">"),
        )).padded();
        let inequality = type_expr.clone()
            .then(op_ineq.then(type_expr).repeated())
            .foldl(|left, (op_str, right)| Expression::Inequality(Box::new(left), op_str.to_string(), Box::new(right)));

        // Level 6: Equality (=, ~, !=, !~) - Left associative
        let op_eq = choice((
            just("=").to("="),
            just("~").to("~"),
            just("!=").to("!="),
            just("!~").to("!~"),
        )).padded();
        let equality = inequality.clone()
            .then(op_eq.then(inequality).repeated())
            .foldl(|left, (op_str, right)| Expression::Equality(Box::new(left), op_str.to_string(), Box::new(right)));

        // Level 7: Membership (in, contains) - Left associative
        let op_mem = choice((
            text::keyword("in").to("in"),
            text::keyword("contains").to("contains"),
        )).padded();
        let membership = equality.clone()
            .then(op_mem.then(equality).repeated())
            .foldl(|left, (op_str, right)| Expression::Membership(Box::new(left), op_str.to_string(), Box::new(right)));

        // Level 8: Logical AND (and) - Left associative
        let op_and = text::keyword("and").padded();
        let logical_and = membership.clone()
            .then(op_and.then(membership).repeated())
            .foldl(|left, (_, right)| Expression::And(Box::new(left), Box::new(right)));

        // Level 9: Logical OR/XOR (or, xor) - Left associative
        let op_or = choice((
            text::keyword("or").to("or"),
            text::keyword("xor").to("xor"),
        )).padded();
        let logical_or = logical_and.clone()
            .then(op_or.then(logical_and).repeated())
            .foldl(|left, (op_str, right)| Expression::Or(Box::new(left), op_str.to_string(), Box::new(right)));

        // Level 10: Implies (implies) - Right associative (or handle as non-assoc if simpler)
        // Let's treat it as left-associative for simplicity, though the spec implies right-to-left evaluation.
        // A proper right-associative fold might be needed if complex implies chains are common.
        let op_implies = text::keyword("implies").padded();
        let implies = logical_or.clone()
            .then(op_implies.then(logical_or).repeated())
            .foldl(|left, (_, right)| Expression::Implies(Box::new(left), Box::new(right)));

        // The final expression parser is the one with the lowest precedence
        implies
    })
    .then_ignore(end())
}
