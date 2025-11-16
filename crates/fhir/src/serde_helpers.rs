use serde::{
    Serialize,
    ser::{self, Serializer},
};

#[derive(Clone, Copy)]
struct ContentFound;

impl std::fmt::Debug for ContentFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ContentFound")
    }
}

impl std::fmt::Display for ContentFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("non-empty content detected")
    }
}

impl std::error::Error for ContentFound {}

impl ser::Error for ContentFound {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        ContentFound
    }
}

struct ContentDetector;

struct SequenceDetector<'a> {
    detector: &'a mut ContentDetector,
}

impl<'a> ser::SerializeSeq for SequenceDetector<'a> {
    type Ok = ();
    type Error = ContentFound;

    fn serialize_element<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        value.serialize(&mut *self.detector)
    }

    fn end(self) -> Result<(), Self::Error> {
        Err(ContentFound)
    }
}

impl<'a> ser::SerializeTuple for SequenceDetector<'a> {
    type Ok = ();
    type Error = ContentFound;

    fn serialize_element<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        value.serialize(&mut *self.detector)
    }

    fn end(self) -> Result<(), Self::Error> {
        Err(ContentFound)
    }
}

impl<'a> ser::SerializeTupleStruct for SequenceDetector<'a> {
    type Ok = ();
    type Error = ContentFound;

    fn serialize_field<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        value.serialize(&mut *self.detector)
    }

    fn end(self) -> Result<(), Self::Error> {
        Err(ContentFound)
    }
}

impl<'a> ser::SerializeTupleVariant for SequenceDetector<'a> {
    type Ok = ();
    type Error = ContentFound;

    fn serialize_field<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        value.serialize(&mut *self.detector)
    }

    fn end(self) -> Result<(), Self::Error> {
        Err(ContentFound)
    }
}

struct MapDetector<'a> {
    _detector: &'a mut ContentDetector,
}

impl<'a> ser::SerializeMap for MapDetector<'a> {
    type Ok = ();
    type Error = ContentFound;

    fn serialize_key<T: ?Sized>(&mut self, _: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        Err(ContentFound)
    }

    fn serialize_value<T: ?Sized>(&mut self, _: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        Ok(())
    }

    fn serialize_entry<K: ?Sized, V: ?Sized>(&mut self, _: &K, _: &V) -> Result<(), Self::Error>
    where
        K: Serialize,
        V: Serialize,
    {
        Err(ContentFound)
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct StructDetector<'a> {
    detector: &'a mut ContentDetector,
    saw_field: bool,
}

impl<'a> ser::SerializeStruct for StructDetector<'a> {
    type Ok = ();
    type Error = ContentFound;

    fn serialize_field<T: ?Sized>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        match value.serialize(&mut *self.detector) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.saw_field = true;
                Err(err)
            }
        }
    }

    fn end(self) -> Result<(), Self::Error> {
        if self.saw_field {
            Err(ContentFound)
        } else {
            Ok(())
        }
    }
}

impl<'a> ser::SerializeStructVariant for StructDetector<'a> {
    type Ok = ();
    type Error = ContentFound;

    fn serialize_field<T: ?Sized>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        match value.serialize(&mut *self.detector) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.saw_field = true;
                Err(err)
            }
        }
    }

    fn end(self) -> Result<(), Self::Error> {
        Err(ContentFound)
    }
}

impl<'a> Serializer for &'a mut ContentDetector {
    type Ok = ();
    type Error = ContentFound;
    type SerializeSeq = SequenceDetector<'a>;
    type SerializeTuple = SequenceDetector<'a>;
    type SerializeTupleStruct = SequenceDetector<'a>;
    type SerializeTupleVariant = SequenceDetector<'a>;
    type SerializeMap = MapDetector<'a>;
    type SerializeStruct = StructDetector<'a>;
    type SerializeStructVariant = StructDetector<'a>;

    fn serialize_bool(self, _value: bool) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_i8(self, _value: i8) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_i16(self, _value: i16) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_i32(self, _value: i32) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_i64(self, _value: i64) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_i128(self, _value: i128) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_u8(self, _value: u8) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_u16(self, _value: u16) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_u32(self, _value: u32) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_u64(self, _value: u64) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_u128(self, _value: u128) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_f32(self, _value: f32) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_f64(self, _value: f64) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_char(self, _value: char) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_str(self, _value: &str) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_none(self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn serialize_some<T: ?Sized>(self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<(), Self::Error> {
        Err(ContentFound)
    }

    fn serialize_newtype_struct<T: ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        Err(ContentFound)
    }

    fn serialize_seq(self, _length: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(SequenceDetector { detector: self })
    }

    fn serialize_tuple(self, _length: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(SequenceDetector { detector: self })
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(SequenceDetector { detector: self })
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(SequenceDetector { detector: self })
    }

    fn serialize_map(self, _length: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(MapDetector { _detector: self })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(StructDetector {
            detector: self,
            saw_field: false,
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Err(ContentFound)
    }
}

pub(crate) fn has_non_empty_content<T>(value: &T) -> bool
where
    T: Serialize,
{
    let mut detector = ContentDetector;
    value.serialize(&mut detector).is_err()
}
