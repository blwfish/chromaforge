//! Does a value hold a number the catalog can't save? JSON has no NaN or infinity: `serde_json` writes them as
//! `null`, which a field of a number type then fails to read back, so a photo record holding one is skipped at the
//! next open (the photo and its edits are gone) and a JSON snapshot holding one doesn't open at all. One check, used
//! where ops enter the catalog ([`crate::Catalog::apply`]) and by every writer of the whole catalog (the v4
//! checkpoint, the JSON snapshot), so none of them can write what the next open drops.
//!
//! The check walks the value as `serde` serializes it, without building any output. A non-finite number directly
//! inside `Some` is let through: it is written as `null`, which reads back as `None` (the value is lost, the record
//! is not), as it always has been.

use serde::Serialize;
use serde::ser;

/// The struct field holding the first non-finite number in `v` (`"?"` when it isn't in a named field), or `None`
/// when every number in it is finite or directly inside `Some`. A value whose own `Serialize` fails counts as
/// finite here: the writer that serializes it reports that error.
pub(crate) fn non_finite_field<T: Serialize + ?Sized>(v: &T) -> Option<&'static str> {
    let mut c = Check { field: None, in_some: false };
    match v.serialize(&mut c) {
        Err(Found::NonFinite(field)) => Some(field),
        Ok(()) | Err(Found::Other) => None,
    }
}

#[derive(Debug)]
enum Found {
    NonFinite(&'static str),
    /// The value's own `Serialize` failed.
    Other,
}

impl std::fmt::Display for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Found::NonFinite(field) => write!(f, "`{field}` is not a finite number"),
            Found::Other => f.write_str("not serializable"),
        }
    }
}

impl std::error::Error for Found {}

impl ser::Error for Found {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Found::Other
    }
}

struct Check {
    /// The struct field being walked.
    field: Option<&'static str>,
    /// The value being walked is directly inside `Some`.
    in_some: bool,
}

impl Check {
    fn number(&mut self, finite: bool) -> Result<(), Found> {
        let in_some = std::mem::take(&mut self.in_some);
        if finite || in_some { Ok(()) } else { Err(Found::NonFinite(self.field.unwrap_or("?"))) }
    }
    /// Anything other than a number: whatever it holds is no longer directly inside `Some`.
    fn other(&mut self) -> Result<(), Found> {
        self.in_some = false;
        Ok(())
    }
    fn nested(&mut self) -> Result<&mut Self, Found> {
        self.in_some = false;
        Ok(self)
    }
}

impl<'a> ser::Serializer for &'a mut Check {
    type Ok = ();
    type Error = Found;
    type SerializeSeq = &'a mut Check;
    type SerializeTuple = &'a mut Check;
    type SerializeTupleStruct = &'a mut Check;
    type SerializeTupleVariant = &'a mut Check;
    type SerializeMap = &'a mut Check;
    type SerializeStruct = &'a mut Check;
    type SerializeStructVariant = &'a mut Check;

    fn serialize_f32(self, v: f32) -> Result<(), Found> {
        self.number(v.is_finite())
    }
    fn serialize_f64(self, v: f64) -> Result<(), Found> {
        self.number(v.is_finite())
    }
    fn serialize_bool(self, _: bool) -> Result<(), Found> {
        self.other()
    }
    fn serialize_i8(self, _: i8) -> Result<(), Found> {
        self.other()
    }
    fn serialize_i16(self, _: i16) -> Result<(), Found> {
        self.other()
    }
    fn serialize_i32(self, _: i32) -> Result<(), Found> {
        self.other()
    }
    fn serialize_i64(self, _: i64) -> Result<(), Found> {
        self.other()
    }
    fn serialize_i128(self, _: i128) -> Result<(), Found> {
        self.other()
    }
    fn serialize_u8(self, _: u8) -> Result<(), Found> {
        self.other()
    }
    fn serialize_u16(self, _: u16) -> Result<(), Found> {
        self.other()
    }
    fn serialize_u32(self, _: u32) -> Result<(), Found> {
        self.other()
    }
    fn serialize_u64(self, _: u64) -> Result<(), Found> {
        self.other()
    }
    fn serialize_u128(self, _: u128) -> Result<(), Found> {
        self.other()
    }
    fn serialize_char(self, _: char) -> Result<(), Found> {
        self.other()
    }
    fn serialize_str(self, _: &str) -> Result<(), Found> {
        self.other()
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<(), Found> {
        self.other()
    }
    fn serialize_none(self) -> Result<(), Found> {
        self.other()
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<(), Found> {
        self.in_some = true;
        let r = v.serialize(&mut *self);
        self.in_some = false;
        r
    }
    fn serialize_unit(self) -> Result<(), Found> {
        self.other()
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Found> {
        self.other()
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<(), Found> {
        self.other()
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, v: &T) -> Result<(), Found> {
        // (a newtype is written as what it holds: `Some(Meters(NaN))` reads back as `None` too)
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(self, _: &'static str, _: u32, _: &'static str, v: &T) -> Result<(), Found> {
        self.in_some = false;
        v.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Found> {
        self.nested()
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Found> {
        self.nested()
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, Found> {
        self.nested()
    }
    fn serialize_tuple_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Self::SerializeTupleVariant, Found> {
        self.nested()
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Found> {
        self.nested()
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Found> {
        self.nested()
    }
    fn serialize_struct_variant(self, _: &'static str, _: u32, _: &'static str, _: usize) -> Result<Self::SerializeStructVariant, Found> {
        self.nested()
    }
    fn collect_str<T: std::fmt::Display + ?Sized>(self, _: &T) -> Result<(), Found> {
        self.other()
    }
}

impl ser::SerializeSeq for &mut Check {
    type Ok = ();
    type Error = Found;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Found> {
        v.serialize(&mut **self)
    }
    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeTuple for &mut Check {
    type Ok = ();
    type Error = Found;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Found> {
        v.serialize(&mut **self)
    }
    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeTupleStruct for &mut Check {
    type Ok = ();
    type Error = Found;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Found> {
        v.serialize(&mut **self)
    }
    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeTupleVariant for &mut Check {
    type Ok = ();
    type Error = Found;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Found> {
        v.serialize(&mut **self)
    }
    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeMap for &mut Check {
    type Ok = ();
    type Error = Found;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, k: &T) -> Result<(), Found> {
        k.serialize(&mut **self)
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), Found> {
        v.serialize(&mut **self)
    }
    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeStruct for &mut Check {
    type Ok = ();
    type Error = Found;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, v: &T) -> Result<(), Found> {
        let outer = self.field.replace(key);
        let r = v.serialize(&mut **self);
        self.field = outer;
        r
    }
    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeStructVariant for &mut Check {
    type Ok = ();
    type Error = Found;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, v: &T) -> Result<(), Found> {
        let outer = self.field.replace(key);
        let r = v.serialize(&mut **self);
        self.field = outer;
        r
    }
    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::non_finite_field;

    #[derive(serde::Serialize)]
    struct Inner {
        radius: f64,
        ratio: f32,
    }

    #[derive(serde::Serialize)]
    struct Outer {
        name: String,
        inner: Inner,
        list: Vec<f64>,
        maybe: Option<f64>,
        pair: Option<(f64, f64)>,
    }

    fn outer() -> Outer {
        Outer { name: "x".into(), inner: Inner { radius: 1.0, ratio: 0.5 }, list: vec![0.0, -0.0], maybe: None, pair: None }
    }

    #[test]
    fn finite_numbers_pass_however_large_or_small() {
        for v in [0.0, -0.0, f64::MAX, f64::MIN, f64::MIN_POSITIVE, f64::MIN_POSITIVE / 2.0, f64::EPSILON] {
            assert_eq!(non_finite_field(&v), None, "{v}");
            assert_eq!(non_finite_field(&Outer { list: vec![v], ..outer() }), None, "{v}");
        }
        assert_eq!(non_finite_field(&f32::MAX), None);
        assert_eq!(non_finite_field(&outer()), None);
    }

    #[test]
    fn nan_and_both_infinities_are_found_and_named() {
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let o = Outer { inner: Inner { radius: v, ratio: 0.5 }, ..outer() };
            assert_eq!(non_finite_field(&o), Some("radius"), "{v}");
            assert_eq!(non_finite_field(&Outer { list: vec![1.0, v], ..outer() }), Some("list"), "{v}");
        }
        assert_eq!(non_finite_field(&Outer { inner: Inner { radius: 1.0, ratio: f32::NAN }, ..outer() }), Some("ratio"));
        assert_eq!(non_finite_field(&f64::NAN), Some("?"), "a bare number has no field name");
    }

    /// `Some(NaN)` is written as `null`, which reads back as `None`: let through. A NaN deeper inside `Some` (a
    /// tuple, a struct) is not: `null` there fails to read back.
    #[test]
    fn a_number_directly_inside_some_is_let_through_but_not_one_deeper() {
        assert_eq!(non_finite_field(&Outer { maybe: Some(f64::NAN), ..outer() }), None);
        assert_eq!(non_finite_field(&Outer { pair: Some((f64::NAN, 0.0)), ..outer() }), Some("pair"));
        assert_eq!(non_finite_field(&Outer { pair: Some((0.0, f64::INFINITY)), ..outer() }), Some("pair"));
        assert_eq!(non_finite_field(&Some(vec![f64::NAN])), Some("?"));
        // and the exemption ends with the value: a later field is checked as usual
        assert_eq!(non_finite_field(&(Some(1.0), f64::NAN)), Some("?"));
        // (and these read back as described)
        let back: Option<f64> = serde_json::from_str(&serde_json::to_string(&Some(f64::NAN)).unwrap()).unwrap();
        assert_eq!(back, None);
        assert!(serde_json::from_str::<Option<(f64, f64)>>(&serde_json::to_string(&Some((f64::NAN, 0.0))).unwrap()).is_err());
    }
}
