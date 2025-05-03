use chumsky::error::Simple;
use chumsky::prelude::*;
use chumsky::Parser;
use rust_decimal::Decimal;
use rust_decimal_macros::dec; // For potential default values if needed
use std::fmt;
use std::str::FromStr;

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
    /* .padded() */; // Remove padding here, let number/integer padding handle space

    // Quantity needs to be a term-level construct to work in expressions
    // Allow either an integer or a number, followed by optional whitespace, then the unit.
    // Remove .padded() from integer/number here and handle padding explicitly.
    let integer_no_pad = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(1)
        .collect::<String>()
        .validate(|digits, span, emit| match i64::from_str(&digits) {
            Ok(n) => Literal::Integer(n),
            Err(_) => {
                emit(Simple::custom(span, format!("Invalid integer: {}", digits)));
                Literal::Integer(0)
            }
        });

    let number_no_pad = filter::<_, _, Simple<char>>(|c: &char| c.is_ascii_digit())
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
                Ok(decimal) => Literal::Number(decimal),
                Err(_) => {
                    emit(Simple::custom(span, format!("Invalid number: {}", num_str)));
                    Literal::Number(dec!(0))
                }
            }
        });

    let quantity = choice((integer_no_pad, number_no_pad))
        .padded() // Allow padding around the number part
        .then_ignore(text::whitespace().or_not()) // Explicitly consume optional whitespace *after* number
        .then(unit) // Then parse the unit (which expects no leading padding now)
        .map(|(num_literal, u)| {
            match num_literal {
                Literal::Integer(i) => {
                    // Convert integer to Decimal for Quantity representation
                    Literal::Quantity(Decimal::from(i), Some(u))
                }
                Literal::Number(d) => Literal::Quantity(d, Some(u)),
                _ => {
                    // This case should ideally not be reachable
                    emit_error("Expected Literal::Integer or Literal::Number in quantity parser, got other literal type.");
                    Literal::Quantity(dec!(0), Some(u)) // Default value
                }
            }
        });

    // Helper function to emit errors (replace with actual logging/error handling if needed)
    fn emit_error(message: &str) {
        eprintln!("Parser Error: {}", message);
    }

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

    // Order matters: try integer before number to avoid ambiguity
    let literal = choice((
        null,
        boolean,
        string,
        quantity, // Quantity uses number, so must come after integer
        number,   // Number requires '.', so must come after integer
        integer,  // Try integer first
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
    let expr_parser = recursive(|expr| {
        // Function parameters
        let param_list = expr
            .clone()
            .separated_by(just(',').padded())
            .collect::<Vec<_>>()
            .boxed();

        // Function invocation
        let function = identifier
            .clone()
            .then(
                just('(')
                    .ignore_then(param_list.clone().or_not().map(|p| p.unwrap_or_default()))
                    .then_ignore(just(')')),
            )
            .map(|(name, params)| Invocation::Function(name, params))
            .boxed();

        // Member invocation
        let invocation = choice((
            function.clone(),
            identifier.clone().map(Invocation::Member),
            just("$this").to(Invocation::This),
            just("$index").to(Invocation::Index),
            just("$total").to(Invocation::Total),
        ))
        .boxed();

        // Define parsers for terms
        let term = {
            // Term - following the grammar rule for 'term'
            choice((
                invocation.clone().map(Term::Invocation),
                literal.clone(),
                external_constant.clone(),
                expr.clone()
                    .delimited_by(just('('), just(')'))
                    .map(|e| Term::Parenthesized(Box::new(e))),
            ))
            .map(Expression::Term)
            .boxed()
        };

        // Now define the expression parser that directly mirrors the grammar structure
        // Start with the base term expression
        let expr_base = term.clone();

        // Define all the operation parsers directly without the closure
        // This avoids the lifetime issues with the previous approach

        // Define a type for our operation functions
        type OpFn = Box<dyn Fn(Expression) -> Expression>;
        
        // Create a vector to hold all our operation parsers
        let mut operations = Vec::<BoxedParser<'_, char, OpFn, Simple<char>>>::new();

        // Invocation expression: expression '.' invocation
        let invocation_expr = just::<_, _, Simple<char>>('.')
            .ignore_then(invocation.clone())
            .map(move |inv| {
                let inv_clone = inv.clone();
                Box::new(move |left: Expression| {
                    Expression::Invocation(Box::new(left), inv_clone.clone())
                }) as OpFn
            })
            .boxed();
        operations.push(invocation_expr);

        // Indexer expression: expression '[' expression ']'
        let indexer_expr = expr
            .clone()
            .delimited_by(
                just::<_, _, Simple<char>>('['),
                just::<_, _, Simple<char>>(']'),
            )
            .map(move |idx| {
                let idx_clone = idx.clone();
                Box::new(move |left: Expression| {
                    Expression::Indexer(Box::new(left), Box::new(idx_clone.clone()))
                }) as OpFn
            })
            .boxed();
        operations.push(indexer_expr);

        // Polarity expression: ('+' | '-') expression
        // This is handled separately as a prefix operator

        // Multiplicative expression: expression ('*' | '/' | 'div' | 'mod') expression
        let multiplicative_expr = choice((
            just::<_, _, Simple<char>>('*').to("*"),
            just::<_, _, Simple<char>>('/').to("/"),
            just::<_, _, Simple<char>>("div").padded().to("div"),
            just::<_, _, Simple<char>>("mod").padded().to("mod"),
        ))
        .padded()
        .then(expr.clone())
        .map(move |(op, right)| {
            let op_str = op.to_string();
            let right_clone = right.clone();
            Box::new(move |left: Expression| {
                Expression::Multiplicative(
                    Box::new(left),
                    op_str.clone(),
                    Box::new(right_clone.clone()),
                )
            }) as OpFn
        })
        .boxed();
        operations.push(multiplicative_expr);

        // Additive expression: expression ('+' | '-' | '&') expression
        let additive_expr = choice((
            just::<_, _, Simple<char>>('+').to("+"),
            just::<_, _, Simple<char>>('-').to("-"),
            just::<_, _, Simple<char>>('&').to("&"),
        ))
        .padded()
        .then(expr.clone())
        .map(move |(op, right)| {
            let op_str = op.to_string();
            let right_clone = right.clone();
            Box::new(move |left: Expression| {
                Expression::Additive(
                    Box::new(left),
                    op_str.clone(),
                    Box::new(right_clone.clone()),
                )
            }) as OpFn
        })
        .boxed();
        operations.push(additive_expr);

        // Type expression: expression ('is' | 'as') typeSpecifier
        let type_expr = choice((
            just::<_, _, Simple<char>>("is").padded().map(|_| "is"),
            just::<_, _, Simple<char>>("as").padded().map(|_| "as"),
        ))
        .then(qualified_identifier.clone())
        .map(move |(op, type_spec)| {
            let op_str = op.to_string();
            let type_spec_clone = type_spec.clone();
            Box::new(move |left: Expression| {
                Expression::Type(Box::new(left), op_str.clone(), type_spec_clone.clone())
            }) as OpFn
        })
        .boxed();
        operations.push(type_expr);

        // Union expression: expression '|' expression
        let union_expr = just::<_, _, Simple<char>>('|')
            .padded()
            .ignore_then(expr.clone())
            .map(move |right| {
                let right_clone = right.clone();
                Box::new(move |left: Expression| {
                    Expression::Union(Box::new(left), Box::new(right_clone.clone()))
                }) as OpFn
            })
            .boxed();
        operations.push(union_expr);

        // Inequality expression: expression ('<=' | '<' | '>' | '>=') expression
        let inequality_expr = choice((
            just::<_, _, Simple<char>>("<=").to("<="),
            just::<_, _, Simple<char>>("<").to("<"),
            just::<_, _, Simple<char>>(">=").to(">="),
            just::<_, _, Simple<char>>(">").to(">"),
        ))
        .padded()
        .then(expr.clone())
        .map(move |(op, right)| {
            let op_str = op.to_string();
            let right_clone = right.clone();
            Box::new(move |left: Expression| {
                Expression::Inequality(
                    Box::new(left),
                    op_str.clone(),
                    Box::new(right_clone.clone()),
                )
            }) as OpFn
        })
        .boxed();
        operations.push(inequality_expr);

        // Equality expression: expression ('=' | '~' | '!=' | '!~') expression
        let equality_expr = choice((
            just::<_, _, Simple<char>>("=").to("="),
            just::<_, _, Simple<char>>("~").to("~"),
            just::<_, _, Simple<char>>("!=").to("!="),
            just::<_, _, Simple<char>>("!~").to("!~"),
        ))
        .padded()
        .then(expr.clone())
        .map(move |(op, right)| {
            let op_str = op.to_string();
            let right_clone = right.clone();
            Box::new(move |left: Expression| {
                Expression::Equality(
                    Box::new(left),
                    op_str.clone(),
                    Box::new(right_clone.clone()),
                )
            }) as OpFn
        })
        .boxed();
        operations.push(equality_expr);

        // Membership expression: expression ('in' | 'contains') expression
        let membership_expr = choice((
            just::<_, _, Simple<char>>("in").padded().to("in"),
            just::<_, _, Simple<char>>("contains")
                .padded()
                .to("contains"),
        ))
        .padded()
        .then(expr.clone())
        .map(move |(op, right)| {
            let op_str = op.to_string();
            let right_clone = right.clone();
            Box::new(move |left: Expression| {
                Expression::Membership(
                    Box::new(left),
                    op_str.clone(),
                    Box::new(right_clone.clone()),
                )
            }) as OpFn
        })
        .boxed();
        operations.push(membership_expr);

        // And expression: expression 'and' expression
        let and_expr = just::<_, _, Simple<char>>("and")
            .padded()
            .ignore_then(expr.clone())
            .map(move |right| {
                let right_clone = right.clone();
                Box::new(move |left: Expression| {
                    Expression::And(Box::new(left), Box::new(right_clone.clone()))
                }) as OpFn
            })
            .boxed();
        operations.push(and_expr);

        // Or expression: expression ('or' | 'xor') expression
        let or_expr = choice((
            just::<_, _, Simple<char>>("or").padded().to("or"),
            just::<_, _, Simple<char>>("xor").padded().to("xor"),
        ))
        .padded()
        .then(expr.clone())
        .map(move |(op, right)| {
            let op_str = op.to_string();
            let right_clone = right.clone();
            Box::new(move |left: Expression| {
                Expression::Or(
                    Box::new(left),
                    op_str.clone(),
                    Box::new(right_clone.clone()),
                )
            }) as OpFn
        })
        .boxed();
        operations.push(or_expr);

        // Implies expression: expression 'implies' expression
        let implies_expr = just::<_, _, Simple<char>>("implies")
            .padded()
            .ignore_then(expr.clone())
            .map(move |right| {
                let right_clone = right.clone();
                Box::new(move |left: Expression| {
                    Expression::Implies(Box::new(left), Box::new(right_clone.clone()))
                }) as OpFn
            })
            .boxed();
        operations.push(implies_expr);

        // Combine all operation parsers using choice
        let operation_parser = choice(operations);

        // Apply operations to the base expression
        let expr_with_operations =
            expr_base
                .clone()
                .then(operation_parser.repeated())
                .map(|(base, operations)| {
                    operations.into_iter().fold(base, |acc, operation| {
                        operation(acc)
                    })
                });

        // Handle polarity expressions separately since they're prefix operators
        let polarity_expr = choice((
            just::<_, _, Simple<char>>('+').to('+'),
            just::<_, _, Simple<char>>('-').to('-'),
        ))
        .padded()
        .then(expr.clone())
        .map(|(op, expr)| Expression::Polarity(op, Box::new(expr)))
        .or(expr_with_operations);

        polarity_expr.boxed() // Return the final parser definition from the closure
    });

    // Apply then_ignore(end()) to the fully defined recursive parser
    expr_parser.then_ignore(end())
}
