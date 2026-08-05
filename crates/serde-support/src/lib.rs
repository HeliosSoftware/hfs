// Serde traits used in custom Deserialize implementations

/// Helper that accepts either a single value or an array when deserializing.
///
/// FHIR allows most repeatable elements to appear either once or multiple times
/// depending on the instance's actual cardinality. While JSON carries enough
/// structure (`[]` vs scalar) so serde can infer that automatically, the XML
/// stream does not embed the schema-driven cardinality constraints. During
/// XML deserialization we therefore wrap every field with a `min > 0` upper
/// bound in `SingleOrVec` so we can accept both the single-element case and
/// the repeated-element case without schema knowledge at parse time.
#[derive(Clone, Debug, PartialEq)]
pub struct SingleOrVec<T>(Vec<T>);

impl<T> AsRef<[T]> for SingleOrVec<T> {
    #[inline]
    fn as_ref(&self) -> &[T] {
        &self.0
    }
}

impl<T> From<SingleOrVec<T>> for Vec<T> {
    #[inline]
    fn from(wrapper: SingleOrVec<T>) -> Self {
        wrapper.0
    }
}

impl<T> Default for SingleOrVec<T> {
    #[inline]
    fn default() -> Self {
        SingleOrVec(Vec::new())
    }
}

// JSON-only: delegate directly to Vec<T> — no deserialize_any overhead
#[cfg(not(feature = "xml"))]
impl<'de, T> serde::Deserialize<'de> for SingleOrVec<T>
where
    T: serde::Deserialize<'de>,
{
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Vec::<T>::deserialize(deserializer).map(SingleOrVec)
    }
}

// XML+JSON: uses deserialize_any to handle both single values and arrays
#[cfg(feature = "xml")]
impl<'de, T> serde::Deserialize<'de> for SingleOrVec<T>
where
    T: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SingleOrVecVisitor<T>(std::marker::PhantomData<T>);

        impl<'de, T> serde::de::Visitor<'de> for SingleOrVecVisitor<T>
        where
            T: serde::Deserialize<'de>,
        {
            type Value = SingleOrVec<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a single value or a sequence")
            }

            // High performance path for JSON arrays or repeated XML tags
            #[inline]
            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let values = serde::Deserialize::deserialize(
                    serde::de::value::SeqAccessDeserializer::new(seq),
                )?;
                Ok(SingleOrVec(values))
            }

            // Path for single XML elements (map = object with fields)
            #[inline]
            fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                let value =
                    deserialize_single_value(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(SingleOrVec(vec![value]))
            }

            // Path for JSON scalars or XML text-only elements
            #[inline]
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let value = deserialize_from_str(v).map_err(serde::de::Error::custom)?;
                Ok(SingleOrVec(vec![value]))
            }

            #[inline]
            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let value = deserialize_single_value(serde::de::value::BoolDeserializer::new(v))?;
                Ok(SingleOrVec(vec![value]))
            }

            #[inline]
            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let value = deserialize_single_value(serde::de::value::I64Deserializer::new(v))?;
                Ok(SingleOrVec(vec![value]))
            }

            #[inline]
            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let value = deserialize_single_value(serde::de::value::U64Deserializer::new(v))?;
                Ok(SingleOrVec(vec![value]))
            }

            #[inline]
            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let value = deserialize_single_value(serde::de::value::F64Deserializer::new(v))?;
                Ok(SingleOrVec(vec![value]))
            }
        }

        deserializer.deserialize_any(SingleOrVecVisitor(std::marker::PhantomData))
    }
}

/// Accepts either JSON primitive values or XML element structures with metadata.
///
/// **JSON Format**: Primitive values come through as scalars, metadata merged from `_field` by macro.
///   - `"birthDate": "1970-03-30"` → `Primitive("1970-03-30")` (String directly)
///   - Metadata in `_field` is handled separately by the generated macro code
///
/// **XML Format**: All primitives are elements with inline metadata, no `_field` exists.
///   - `<birthDate value="1970-03-30"/>` → `Element(Element { value: Some(...), id: None, ... })`
///   - `<birthDate id="x" value="...">` → `Element(Element { value, id, ... })`
///   - `<birthDate id="x" value="..."><extension>...</extension></birthDate>` → `Element` with full metadata
///
/// The custom `Deserialize` impl mirrors the old `#[serde(untagged)]` behavior without buffering:
/// - JSON scalars map to the `Primitive` variant (directly deserialized into the primitive type).
/// - XML element structures (objects with `value`, `id`, `extension`, …) map to the `Element` variant.
/// It avoids serde’s internal `Content` buffering while preserving semantics crucial for primitives
/// with metadata.
///
/// # Type Parameters
/// - `P`: Primitive type (the final deserialized type, e.g. `String`, `i32`, `bool`)
/// - `E`: Element type (struct containing value and metadata fields)
#[derive(Clone, Debug, PartialEq)]
pub enum PrimitiveOrElement<P, E> {
    // Try Element first (more specific - requires object structure)
    Element(E),
    // Fall back to Primitive (catch-all for JSON scalars)
    Primitive(P),
}

// JSON-only: always produces Primitive — no deserialize_any overhead
#[cfg(not(feature = "xml"))]
impl<'de, P, E> serde::Deserialize<'de> for PrimitiveOrElement<P, E>
where
    P: serde::Deserialize<'de>,
{
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        P::deserialize(deserializer).map(PrimitiveOrElement::Primitive)
    }
}

// XML+JSON: uses deserialize_any to distinguish primitives from element objects
#[cfg(feature = "xml")]
impl<'de, P, E> serde::Deserialize<'de> for PrimitiveOrElement<P, E>
where
    P: serde::Deserialize<'de>,
    E: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PrimitiveOrElementVisitor<P, E>(std::marker::PhantomData<(P, E)>);

        impl<'de, P, E> serde::de::Visitor<'de> for PrimitiveOrElementVisitor<P, E>
        where
            P: serde::Deserialize<'de>,
            E: serde::Deserialize<'de>,
        {
            type Value = PrimitiveOrElement<P, E>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a primitive value or an element object")
            }

            #[inline]
            fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                let element = E::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(PrimitiveOrElement::Element(element))
            }

            #[inline]
            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let primitive =
                    deserialize_single_value(serde::de::value::SeqAccessDeserializer::new(seq))?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_str<E2>(self, v: &str) -> Result<Self::Value, E2>
            where
                E2: serde::de::Error,
            {
                let primitive = deserialize_from_str(v).map_err(serde::de::Error::custom)?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_string<E2>(self, v: String) -> Result<Self::Value, E2>
            where
                E2: serde::de::Error,
            {
                let primitive = deserialize_from_str(&v).map_err(serde::de::Error::custom)?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_bool<E2>(self, v: bool) -> Result<Self::Value, E2>
            where
                E2: serde::de::Error,
            {
                let primitive =
                    deserialize_single_value(serde::de::value::BoolDeserializer::new(v))?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_i64<E2>(self, v: i64) -> Result<Self::Value, E2>
            where
                E2: serde::de::Error,
            {
                let primitive =
                    deserialize_single_value(serde::de::value::I64Deserializer::new(v))?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_u64<E2>(self, v: u64) -> Result<Self::Value, E2>
            where
                E2: serde::de::Error,
            {
                let primitive =
                    deserialize_single_value(serde::de::value::U64Deserializer::new(v))?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_f64<E2>(self, v: f64) -> Result<Self::Value, E2>
            where
                E2: serde::de::Error,
            {
                let primitive =
                    deserialize_single_value(serde::de::value::F64Deserializer::new(v))?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_none<E2>(self) -> Result<Self::Value, E2>
            where
                E2: serde::de::Error,
            {
                let primitive = P::deserialize(serde::de::value::UnitDeserializer::new())?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_unit<E2>(self) -> Result<Self::Value, E2>
            where
                E2: serde::de::Error,
            {
                let primitive = P::deserialize(serde::de::value::UnitDeserializer::new())?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
            where
                D2: serde::Deserializer<'de>,
            {
                let primitive = deserialize_single_value(deserializer)?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_newtype_struct<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
            where
                D2: serde::Deserializer<'de>,
            {
                let primitive = deserialize_single_value(deserializer)?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_enum<D2>(self, data: D2) -> Result<Self::Value, D2::Error>
            where
                D2: serde::de::EnumAccess<'de>,
            {
                let primitive =
                    deserialize_single_value(serde::de::value::EnumAccessDeserializer::new(data))?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }

            #[inline]
            fn visit_char<E2>(self, v: char) -> Result<Self::Value, E2>
            where
                E2: serde::de::Error,
            {
                let primitive =
                    deserialize_single_value(serde::de::value::CharDeserializer::new(v))?;
                Ok(PrimitiveOrElement::Primitive(primitive))
            }
        }

        deserializer.deserialize_any(PrimitiveOrElementVisitor(std::marker::PhantomData))
    }
}

#[cfg(feature = "xml")]
#[inline]
fn deserialize_single_value<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    /// Wraps a deserializer so that `Option<T>` values produced from scalars are treated as `Some(T)`.
    struct OptionFriendlyDeserializer<D>(D);

    impl<'de, D> serde::Deserializer<'de> for OptionFriendlyDeserializer<D>
    where
        D: serde::Deserializer<'de>,
    {
        type Error = D::Error;

        #[inline]
        fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: serde::de::Visitor<'de>,
        {
            self.0.deserialize_any(visitor)
        }

        #[inline]
        fn deserialize_enum<V>(
            self,
            name: &'static str,
            variants: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Self::Error>
        where
            V: serde::de::Visitor<'de>,
        {
            self.0.deserialize_enum(name, variants, visitor)
        }

        #[inline]
        fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: serde::de::Visitor<'de>,
        {
            visitor.visit_some(self.0)
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
            bytes byte_buf unit unit_struct newtype_struct seq tuple tuple_struct
            map struct identifier ignored_any
        }
    }
    T::deserialize(OptionFriendlyDeserializer(deserializer))
}

#[cfg(feature = "xml")]
/// Deserializer that wraps a string value and tries to parse it as the requested type.
///
/// XML sends all primitive values as strings. This deserializer enables transparent
/// conversion from strings to bool, integer, and float types during deserialization.
/// It first tries the string directly (works for String targets), and if the target
/// type requests a specific numeric or boolean type, it parses the string accordingly.
struct StringParsingDeserializer<'a>(&'a str);

#[cfg(feature = "xml")]
impl<'de, 'a> serde::Deserializer<'de> for StringParsingDeserializer<'a> {
    type Error = serde::de::value::Error;

    #[inline]
    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        // Try string first (works for String, &str targets)
        visitor.visit_str(self.0)
    }

    #[inline]
    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0 {
            "true" => visitor.visit_bool(true),
            "false" => visitor.visit_bool(false),
            _ => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"true or false",
            )),
        }
    }

    #[inline]
    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<i8>() {
            Ok(v) => visitor.visit_i8(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"an i8 integer",
            )),
        }
    }

    #[inline]
    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<i16>() {
            Ok(v) => visitor.visit_i16(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"an i16 integer",
            )),
        }
    }

    #[inline]
    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<i32>() {
            Ok(v) => visitor.visit_i32(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"an i32 integer",
            )),
        }
    }

    #[inline]
    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<i64>() {
            Ok(v) => visitor.visit_i64(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"an i64 integer",
            )),
        }
    }

    #[inline]
    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<u8>() {
            Ok(v) => visitor.visit_u8(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"a u8 integer",
            )),
        }
    }

    #[inline]
    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<u16>() {
            Ok(v) => visitor.visit_u16(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"a u16 integer",
            )),
        }
    }

    #[inline]
    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<u32>() {
            Ok(v) => visitor.visit_u32(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"a u32 integer",
            )),
        }
    }

    #[inline]
    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<u64>() {
            Ok(v) => visitor.visit_u64(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"a u64 integer",
            )),
        }
    }

    #[inline]
    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<f32>() {
            Ok(v) => visitor.visit_f32(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"an f32 float",
            )),
        }
    }

    #[inline]
    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<f64>() {
            Ok(v) => visitor.visit_f64(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"an f64 float",
            )),
        }
    }

    #[inline]
    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    serde::forward_to_deserialize_any! {
        char str string bytes byte_buf unit unit_struct newtype_struct
        seq tuple tuple_struct map struct enum identifier ignored_any
    }

    #[inline]
    fn deserialize_i128<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<i128>() {
            Ok(v) => visitor.visit_i128(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"an i128 integer",
            )),
        }
    }

    #[inline]
    fn deserialize_u128<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.0.parse::<u128>() {
            Ok(v) => visitor.visit_u128(v),
            Err(_) => Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(self.0),
                &"a u128 integer",
            )),
        }
    }
}

#[cfg(feature = "xml")]
/// Deserializes a value from a string, trying type-specific parsing.
///
/// Used by `PrimitiveOrElement` and `SingleOrVec` when receiving string values
/// from the XML deserializer to convert them into concrete primitive types.
/// Uses `StringParsingDeserializer` directly (not `deserialize_single_value`)
/// because the `OptionFriendlyDeserializer` wrapper would forward type-specific
/// methods like `deserialize_i32` back to `deserialize_any`, losing the parsing.
#[inline]
fn deserialize_from_str<'de, T>(s: &str) -> Result<T, serde::de::value::Error>
where
    T: serde::Deserialize<'de>,
{
    T::deserialize(StringParsingDeserializer(s))
}

/// Helper struct for serializing id and extension metadata for FHIR primitives.
///
/// In FHIR JSON, primitive values can have associated metadata stored in a parallel
/// `_fieldName` object containing an `id` and/or `extension` array.
///
/// This helper is used during serialization to output only the id/extension metadata
/// while the primitive value itself is serialized separately.
///
/// # Type Parameters
/// - `'a`: Lifetime of the borrowed data
/// - `E`: Extension type (varies by FHIR version: R4, R4B, R5, R6)
///
/// # Example
/// ```json
/// {
///   "status": "active",
///   "_status": {
///     "id": "status-1",
///     "extension": [...]
///   }
/// }
/// ```
#[derive(serde::Serialize)]
pub struct IdAndExtensionHelper<'a, E> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: &'a Option<Vec<E>>,
}

/// Helper struct for deserializing id and extension metadata for FHIR primitives.
///
/// This is the owned version of `IdAndExtensionHelper`, used during deserialization
/// to capture id and extension data from the `_fieldName` JSON object.
///
/// # Type Parameters
/// - `E`: Extension type (varies by FHIR version: R4, R4B, R5, R6)
#[derive(Clone, serde::Deserialize, Default)]
pub struct IdAndExtensionOwned<E> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<E>>,
}

/// The raw JSON pieces of a FHIR choice element (`value[x]`, `effective[x]`, …).
///
/// FHIR serialises a choice element as a *type-suffixed* key (`valueQuantity`),
/// optionally paired with an underscore-prefixed sibling (`_valueString`) that
/// carries `id`/`extension` metadata for primitive choices. [`deserialize_choice_parts`]
/// locates that pair and reports which variant matched.
pub struct ChoiceParts {
    /// Index into the `variant_keys` slice that was matched.
    pub index: usize,
    /// The value under the type-suffixed key, when present.
    pub value: Option<serde_json::Value>,
    /// The value under the underscore-prefixed key, when present.
    pub extension: Option<serde_json::Value>,
}

/// Scans a map for the single choice-element key it contains.
///
/// This exists so that the `FhirSerde` derive does not have to expand a
/// per-variant key comparison into every generated `visit_map`. Choice enums
/// carry one variant per permitted FHIR datatype — dozens each — and there are
/// hundreds of such enums per FHIR version, so an inlined scan multiplies out
/// into hundreds of megabytes of near-identical machine code in any binary that
/// deserializes typed resources. Keying the scan on a `&'static [&'static str]`
/// collapses all of it to one function per `MapAccess` implementation.
///
/// Keys that match neither a variant key nor its underscore-prefixed form are
/// ignored, matching the previous generated behaviour.
///
/// # Errors
///
/// Returns an error when a key is not a string, when either the value or the
/// extension key appears twice, when two *different* variant keys are present,
/// or when no variant key is found at all.
pub fn deserialize_choice_parts<'de, A>(
    mut map: A,
    variant_keys: &'static [&'static str],
) -> Result<ChoiceParts, A::Error>
where
    A: serde::de::MapAccess<'de>,
{
    let mut index: Option<usize> = None;
    let mut value: Option<serde_json::Value> = None;
    let mut extension: Option<serde_json::Value> = None;

    while let Some((key, current)) = map.next_entry::<serde_json::Value, serde_json::Value>()? {
        let key = match key {
            serde_json::Value::String(key) => key,
            _ => {
                return Err(serde::de::Error::invalid_type(
                    serde::de::Unexpected::Other("non-string key"),
                    &"a string key",
                ));
            }
        };

        // Exact match first, so a (hypothetical) variant key starting with `_`
        // still wins over being read as another variant's metadata sibling.
        let matched = if let Some(pos) = variant_keys.iter().position(|k| *k == key) {
            if value.is_some() {
                return Err(serde::de::Error::duplicate_field(variant_keys[pos]));
            }
            value = Some(current);
            pos
        } else if let Some(pos) = key
            .strip_prefix('_')
            .and_then(|base| variant_keys.iter().position(|k| *k == base))
        {
            if extension.is_some() {
                // `duplicate_field` requires a `&'static str`, which we do not
                // have for the underscore-prefixed form.
                return Err(serde::de::Error::custom(format!(
                    "duplicate field '{}'",
                    key
                )));
            }
            extension = Some(current);
            pos
        } else {
            continue;
        };

        match index {
            Some(existing) if existing != matched => {
                return Err(serde::de::Error::custom(format!(
                    "Mismatched keys found: {} and {}",
                    variant_keys[existing], key
                )));
            }
            _ => index = Some(matched),
        }
    }

    match index {
        Some(index) => Ok(ChoiceParts {
            index,
            value,
            extension,
        }),
        None => Err(serde::de::Error::custom(format!(
            "Expected one of the variant keys {:?} (or their underscore-prefixed versions) but found none",
            variant_keys
        ))),
    }
}

/// Deserializes the `value`/`id`/`extension` pieces of a primitive choice variant.
///
/// The caller assembles them into its concrete `Element<V, E>` or
/// `DecimalElement<E>`; `V` and `E` are inferred from that assignment. Shared
/// out of the derive for the same code-size reason as [`deserialize_choice_parts`].
///
/// # Errors
///
/// Returns an error when either piece fails to deserialize.
#[allow(clippy::type_complexity)]
pub fn deserialize_choice_element_parts<V, E, Err>(
    value: Option<serde_json::Value>,
    extension: Option<serde_json::Value>,
    key: &str,
) -> Result<(Option<V>, Option<String>, Option<Vec<E>>), Err>
where
    V: serde::de::DeserializeOwned,
    E: serde::de::DeserializeOwned,
    Err: serde::de::Error,
{
    let (id, extension) = match extension {
        Some(extension) => {
            let helper: IdAndExtensionOwned<E> =
                serde::Deserialize::deserialize(extension).map_err(|e| {
                    Err::custom(format!("Error deserializing extension _{}: {}", key, e))
                })?;
            (helper.id, helper.extension)
        }
        None => (None, None),
    };

    let value = match value {
        Some(value) => Some(V::deserialize(value).map_err(|e| {
            Err::custom(format!("Error deserializing primitive {}: {}", key, e))
        })?),
        None => None,
    };

    Ok((value, id, extension))
}

/// Deserializes the payload of a non-primitive choice variant.
///
/// `kind` only shapes the error message (`"non-element variant"`,
/// `"tuple variant"`, …). Shared out of the derive for the same code-size
/// reason as [`deserialize_choice_parts`].
///
/// # Errors
///
/// Returns an error when the value is absent or fails to deserialize.
pub fn deserialize_choice_value<T, Err>(
    value: Option<serde_json::Value>,
    key: &'static str,
    kind: &'static str,
) -> Result<T, Err>
where
    T: serde::de::DeserializeOwned,
    Err: serde::de::Error,
{
    let value = value.ok_or_else(|| Err::missing_field(key))?;
    T::deserialize(value)
        .map_err(|e| Err::custom(format!("Error deserializing {} {}: {}", kind, key, e)))
}
