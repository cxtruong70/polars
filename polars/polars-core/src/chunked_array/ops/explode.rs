use crate::prelude::*;
use arrow::bitmap::Bitmap;
use arrow::{array::*, bitmap::MutableBitmap, buffer::Buffer};
use polars_arrow::array::PolarsArray;
use polars_arrow::bit_util::unset_bit_raw;
use polars_arrow::prelude::{FromDataUtf8, ValueSize};
use std::convert::TryFrom;

pub(crate) trait ExplodeByOffsets {
    fn explode_by_offsets(&self, offsets: &[i64]) -> Series;
}

unsafe fn unset_nulls(
    start: usize,
    last: usize,
    validity_values: &Bitmap,
    nulls: &mut Vec<usize>,
    empty_row_idx: &[usize],
) {
    for i in start..last {
        if !validity_values.get_bit_unchecked(i) {
            nulls.push(i + empty_row_idx.len());
        }
    }
}

impl<T> ExplodeByOffsets for ChunkedArray<T>
where
    T: PolarsIntegerType,
{
    fn explode_by_offsets(&self, offsets: &[i64]) -> Series {
        debug_assert_eq!(self.chunks.len(), 1);
        let arr = self.downcast_iter().next().unwrap();
        let values = arr.values();

        let mut new_values = Vec::with_capacity(((values.len() as f32) * 1.5) as usize);
        let mut empty_row_idx = vec![];
        let mut nulls = vec![];

        let mut start = offsets[0] as usize;
        let mut last = start;

        // we check all the offsets and in the case a consecutive offset is the same,
        // e.g. 0, 1, 4, 4, 6
        // the 4 4, means that that is an empty row.
        // the empty row will be replaced with a None value.
        //
        // below we memcpy as much as possible and for the empty rows we add a default value
        // that value will later be masked out by the validity bitmap

        // in the case that the value array has got null values, we need to check every validity
        // value and collect the indices.
        // because the length of the array is not known, we first collect the null indexes, offset
        // with the insertion of empty rows (as None) and later create a validity bitmap
        if arr.has_validity() {
            let validity_values = arr.validity().unwrap();

            for &o in &offsets[1..] {
                let o = o as usize;
                if o == last {
                    if start != last {
                        new_values.extend_from_slice(&values[start..last]);
                        // Safety:
                        // we are in bounds
                        unsafe {
                            unset_nulls(start, last, validity_values, &mut nulls, &empty_row_idx)
                        }
                    }

                    empty_row_idx.push(o + empty_row_idx.len());
                    new_values.push(T::Native::default());
                    start = o;
                }
                last = o;
            }

            // final null check
            // Safety:
            // we are in bounds
            unsafe { unset_nulls(start, last, validity_values, &mut nulls, &empty_row_idx) }
        } else {
            for &o in &offsets[1..] {
                let o = o as usize;
                if o == last {
                    if start != last {
                        new_values.extend_from_slice(&values[start..last])
                    }

                    empty_row_idx.push(o + empty_row_idx.len());
                    new_values.push(T::Native::default());
                    start = o;
                }
                last = o;
            }
        }

        new_values.extend_from_slice(&values[start..]);

        let mut validity = MutableBitmap::with_capacity(new_values.len());
        validity.extend_constant(new_values.len(), true);
        let validity_slice = validity.as_slice().as_ptr() as *mut u8;

        for i in empty_row_idx {
            unsafe { unset_bit_raw(validity_slice, i) }
        }
        for i in nulls {
            unsafe { unset_bit_raw(validity_slice, i) }
        }
        let arr = PrimitiveArray::from_data(
            T::get_dtype().to_arrow(),
            new_values.into(),
            Some(validity.into()),
        );
        Series::try_from((self.name(), Box::new(arr) as ArrayRef)).unwrap()
    }
}

impl ExplodeByOffsets for Float32Chunked {
    fn explode_by_offsets(&self, offsets: &[i64]) -> Series {
        self.apply_as_ints(|s| s.explode_by_offsets(offsets))
    }
}
impl ExplodeByOffsets for Float64Chunked {
    fn explode_by_offsets(&self, offsets: &[i64]) -> Series {
        self.apply_as_ints(|s| s.explode_by_offsets(offsets))
    }
}

impl ExplodeByOffsets for BooleanChunked {
    fn explode_by_offsets(&self, offsets: &[i64]) -> Series {
        debug_assert_eq!(self.chunks.len(), 1);
        let arr = self.downcast_iter().next().unwrap();

        let cap = ((arr.len() as f32) * 1.5) as usize;
        let mut builder = BooleanChunkedBuilder::new(self.name(), cap);

        let mut start = offsets[0] as usize;
        let mut last = start;
        for &o in &offsets[1..] {
            let o = o as usize;
            if o == last {
                if start != last {
                    let vals = arr.slice(start, last - start);
                    let vals_ref = vals.as_any().downcast_ref::<BooleanArray>().unwrap();
                    for val in vals_ref {
                        builder.append_option(val)
                    }
                }
                builder.append_null();
                start = o;
            }
            last = o;
        }
        let vals = arr.slice(start, last - start);
        let vals_ref = vals.as_any().downcast_ref::<BooleanArray>().unwrap();
        for val in vals_ref {
            builder.append_option(val)
        }
        builder.finish().into()
    }
}
impl ExplodeByOffsets for ListChunked {
    fn explode_by_offsets(&self, _offsets: &[i64]) -> Series {
        panic!("cannot explode List of Lists")
    }
}
impl ExplodeByOffsets for Utf8Chunked {
    fn explode_by_offsets(&self, offsets: &[i64]) -> Series {
        debug_assert_eq!(self.chunks.len(), 1);
        let arr = self.downcast_iter().next().unwrap();

        let cap = ((arr.len() as f32) * 1.5) as usize;
        let bytes_size = self.get_values_size();
        let mut builder = Utf8ChunkedBuilder::new(self.name(), cap, bytes_size);

        let mut start = offsets[0] as usize;
        let mut last = start;
        for &o in &offsets[1..] {
            let o = o as usize;
            if o == last {
                if start != last {
                    let vals = arr.slice(start, last - start);
                    let vals_ref = vals.as_any().downcast_ref::<LargeStringArray>().unwrap();
                    for val in vals_ref {
                        builder.append_option(val)
                    }
                }
                builder.append_null();
                start = o;
            }
            last = o;
        }
        let vals = arr.slice(start, last - start);
        let vals_ref = vals.as_any().downcast_ref::<LargeStringArray>().unwrap();
        for val in vals_ref {
            builder.append_option(val)
        }
        builder.finish().into()
    }
}

/// Convert Arrow array offsets to indexes of the original list
pub(crate) fn offsets_to_indexes(offsets: &[i64], capacity: usize) -> Vec<IdxSize> {
    if offsets.is_empty() {
        return vec![];
    }
    let mut idx = Vec::with_capacity(capacity);

    let mut count = 0;
    let mut last_idx = 0;
    let mut previous_empty = false;
    for offset in &offsets[1..] {
        while count < *offset {
            count += 1;
            idx.push(last_idx)
        }
        // Safety:
        // we started iterating from 1, so there is always a previous offset
        // we take the pointer to the previous element and deref that to get
        // the previous offset
        let previous_offset = unsafe { *(offset as *const i64).offset(-1) };

        if !previous_empty && (previous_offset != *offset) {
            last_idx += 1;
        } else {
            count += 1;
            idx.push(last_idx);
            last_idx += 1;
        }
        previous_empty = previous_offset == *offset;
    }
    // last appended index.
    let last_idx = idx[idx.len() - 1];
    for _ in 0..(capacity - count as usize) {
        idx.push(last_idx);
    }
    idx
}

impl ChunkExplode for ListChunked {
    fn explode_and_offsets(&self) -> Result<(Series, Buffer<i64>)> {
        // A list array's memory layout is actually already 'exploded', so we can just take the values array
        // of the list. And we also return a slice of the offsets. This slice can be used to find the old
        // list layout or indexes to expand the DataFrame in the same manner as the 'explode' operation
        let ca = self.rechunk();
        let listarr: &LargeListArray = ca
            .downcast_iter()
            .next()
            .ok_or_else(|| PolarsError::NoData("cannot explode empty list".into()))?;
        let offsets_buf = listarr.offsets().clone();
        let offsets = listarr.offsets().as_slice();
        let mut values = listarr.values().clone();

        // all empty
        if offsets[offsets.len() - 1] == 0 {
            return Ok((
                Series::new_empty(self.name(), &self.inner_dtype()),
                vec![].into(),
            ));
        }

        if !offsets.is_empty() {
            let offset = offsets[0];
            // safety:
            // we are in bounds
            values = unsafe {
                values.slice_unchecked(offset as usize, offsets[offsets.len() - 1] as usize)
            };
        }

        let mut s = if ca.can_fast_explode() {
            Series::try_from((self.name(), values)).unwrap()
        } else {
            // during tests
            // test that this code branch is not hit with list arrays that could be fast exploded
            #[cfg(test)]
            {
                let mut last = offsets[0];
                let mut has_empty = false;
                for &o in &offsets[1..] {
                    if o == last {
                        has_empty = true;
                    }
                    last = o;
                }
                if !has_empty {
                    panic!("could have fast exploded")
                }
            }

            let values = Series::try_from((self.name(), values)).unwrap();
            values.explode_by_offsets(offsets)
        };
        debug_assert_eq!(s.name(), self.name());
        // make sure we restore the logical type
        match self.inner_dtype() {
            #[cfg(feature = "dtype-categorical")]
            DataType::Categorical(rev_map) => {
                let cats = s.u32().unwrap().clone();
                s = CategoricalChunked::from_cats_and_rev_map(cats, rev_map.unwrap()).into_series();
            }
            #[cfg(feature = "dtype-date")]
            DataType::Date => s = s.into_date(),
            #[cfg(feature = "dtype-datetime")]
            DataType::Datetime(tu, tz) => s = s.into_datetime(tu, tz),
            #[cfg(feature = "dtype-duration")]
            DataType::Duration(tu) => s = s.into_duration(tu),
            #[cfg(feature = "dtype-time")]
            DataType::Time => s = s.into_time(),
            _ => {}
        }

        Ok((s, offsets_buf))
    }
}

impl ChunkExplode for Utf8Chunked {
    fn explode_and_offsets(&self) -> Result<(Series, Buffer<i64>)> {
        // A list array's memory layout is actually already 'exploded', so we can just take the values array
        // of the list. And we also return a slice of the offsets. This slice can be used to find the old
        // list layout or indexes to expand the DataFrame in the same manner as the 'explode' operation
        let ca = self.rechunk();
        let array: &Utf8Array<i64> = ca
            .downcast_iter()
            .next()
            .ok_or_else(|| PolarsError::NoData("cannot explode empty str".into()))?;
        let values = array.values();
        let old_offsets = array.offsets().clone();

        // Because the strings are u8 stored but really are utf8 data we need to traverse the utf8 to
        // get the chars indexes
        // Utf8Array guarantees that this holds.
        let str_data = unsafe { std::str::from_utf8_unchecked(values) };

        // iterator over index and chars, we take only the index
        let chars = str_data
            .char_indices()
            .map(|t| t.0 as i64)
            .chain(std::iter::once(str_data.len() as i64));

        let offsets = Buffer::from_iter(chars);

        // the old bitmap doesn't fit on the exploded array, so we need to create a new one.
        let validity = if let Some(validity) = array.validity() {
            let capacity = offsets.len();
            let mut bitmap = MutableBitmap::with_capacity(offsets.len() - 1);

            let mut count = 0;
            let mut last_idx = 0;
            let mut last_valid = validity.get_bit(last_idx);
            for &offset in offsets.iter().skip(1) {
                while count < offset {
                    count += 1;
                    bitmap.push(last_valid);
                }
                last_idx += 1;
                last_valid = validity.get_bit(last_idx);
            }
            for _ in 0..(capacity - count as usize) {
                bitmap.push(last_valid);
            }
            bitmap.into()
        } else {
            None
        };
        let array = unsafe {
            Utf8Array::<i64>::from_data_unchecked_default(offsets, values.clone(), validity)
        };

        let new_arr = Box::new(array) as ArrayRef;

        let s = Series::try_from((self.name(), new_arr)).unwrap();
        Ok((s, old_offsets))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::chunked_array::builder::get_list_builder;

    #[test]
    fn test_explode_list() -> Result<()> {
        let mut builder = get_list_builder(&DataType::Int32, 5, 5, "a")?;

        builder.append_series(&Series::new("", &[1, 2, 3, 3]));
        builder.append_series(&Series::new("", &[1]));
        builder.append_series(&Series::new("", &[2]));

        let ca = builder.finish();
        assert!(ca.can_fast_explode());

        // normal explode
        let exploded = ca.explode()?;
        let out: Vec<_> = exploded.i32()?.into_no_null_iter().collect();
        assert_eq!(out, &[1, 2, 3, 3, 1, 2]);

        // sliced explode
        let exploded = ca.slice(0, 1).explode()?;
        let out: Vec<_> = exploded.i32()?.into_no_null_iter().collect();
        assert_eq!(out, &[1, 2, 3, 3]);

        Ok(())
    }

    #[test]
    fn test_explode_list_nulls() -> Result<()> {
        let ca = Int32Chunked::from_slice_options("", &[None, Some(1), Some(2)]);
        let offsets = &[0, 3, 3];
        let out = ca.explode_by_offsets(offsets);
        assert_eq!(
            Vec::from(out.i32().unwrap()),
            &[None, Some(1), Some(2), None]
        );

        let ca = BooleanChunked::from_slice_options("", &[None, Some(true), Some(false)]);
        let out = ca.explode_by_offsets(offsets);
        assert_eq!(
            Vec::from(out.bool().unwrap()),
            &[None, Some(true), Some(false), None]
        );

        let ca = Utf8Chunked::from_slice_options("", &[None, Some("b"), Some("c")]);
        let out = ca.explode_by_offsets(offsets);
        assert_eq!(
            Vec::from(out.utf8().unwrap()),
            &[None, Some("b"), Some("c"), None]
        );
        Ok(())
    }

    #[test]
    fn test_explode_empty_list() -> Result<()> {
        let mut builder = get_list_builder(&DataType::Int32, 1, 1, "a")?;

        let vals: [i32; 0] = [];

        builder.append_series(&Series::new("", &vals));
        let ca = builder.finish();

        // normal explode
        let exploded = ca.explode()?;
        assert_eq!(exploded.len(), 0);
        assert_eq!(exploded.dtype(), &DataType::Int32);

        Ok(())
    }

    #[test]
    fn test_explode_empty_list_slot() -> Result<()> {
        // primitive
        let mut builder = get_list_builder(&DataType::Int32, 5, 5, "a")?;
        builder.append_series(&Series::new("", &[1i32, 2]));
        builder.append_series(&Int32Chunked::from_slice("", &[]).into_series());
        builder.append_series(&Series::new("", &[3i32]));

        let ca = builder.finish();
        let exploded = ca.explode()?;
        assert_eq!(
            Vec::from(exploded.i32()?),
            &[Some(1), Some(2), None, Some(3)]
        );

        // more primitive
        let mut builder = get_list_builder(&DataType::Int32, 5, 5, "a")?;
        builder.append_series(&Series::new("", &[1i32]));
        builder.append_series(&Int32Chunked::from_slice("", &[]).into_series());
        builder.append_series(&Series::new("", &[2i32]));
        builder.append_series(&Int32Chunked::from_slice("", &[]).into_series());
        builder.append_series(&Series::new("", &[3, 4i32]));

        let ca = builder.finish();
        let exploded = ca.explode()?;
        assert_eq!(
            Vec::from(exploded.i32()?),
            &[Some(1), None, Some(2), None, Some(3), Some(4)]
        );

        // utf8
        let mut builder = get_list_builder(&DataType::Utf8, 5, 5, "a")?;
        builder.append_series(&Series::new("", &["abc"]));
        builder.append_series(
            &<Utf8Chunked as NewChunkedArray<Utf8Type, &str>>::from_slice("", &[]).into_series(),
        );
        builder.append_series(&Series::new("", &["de"]));
        builder.append_series(
            &<Utf8Chunked as NewChunkedArray<Utf8Type, &str>>::from_slice("", &[]).into_series(),
        );
        builder.append_series(&Series::new("", &["fg"]));
        builder.append_series(
            &<Utf8Chunked as NewChunkedArray<Utf8Type, &str>>::from_slice("", &[]).into_series(),
        );

        let ca = builder.finish();
        let exploded = ca.explode()?;
        assert_eq!(
            Vec::from(exploded.utf8()?),
            &[Some("abc"), None, Some("de"), None, Some("fg"), None]
        );

        // boolean
        let mut builder = get_list_builder(&DataType::Boolean, 5, 5, "a")?;
        builder.append_series(&Series::new("", &[true]));
        builder.append_series(&BooleanChunked::from_slice("", &[]).into_series());
        builder.append_series(&Series::new("", &[false]));
        builder.append_series(&BooleanChunked::from_slice("", &[]).into_series());
        builder.append_series(&Series::new("", &[true, true]));

        let ca = builder.finish();
        let exploded = ca.explode()?;
        assert_eq!(
            Vec::from(exploded.bool()?),
            &[Some(true), None, Some(false), None, Some(true), Some(true)]
        );

        Ok(())
    }
}
