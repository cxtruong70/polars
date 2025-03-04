use crate::chunked_array::cast::cast_chunks;
#[cfg(feature = "object")]
use crate::chunked_array::object::extension::polars_extension::PolarsExtension;
use crate::prelude::*;
use arrow::compute::cast::utf8_to_large_utf8;
#[cfg(any(
    feature = "dtype-date",
    feature = "dtype-datetime",
    feature = "dtype-time"
))]
use arrow::temporal_conversions::*;
use polars_arrow::compute::cast::cast;
#[cfg(feature = "dtype-struct")]
use polars_arrow::kernels::concatenate::concatenate_owned_unchecked;
use std::convert::TryFrom;

impl Series {
    /// Takes chunks and a polars datatype and constructs the Series
    /// This is faster than creating from chunks and an arrow datatype because there is no
    /// casting involved
    ///
    /// # Safety
    ///
    /// The caller must ensure that the given `dtype`'s physical type matches all the `ArrayRef` dtypes.
    pub(crate) unsafe fn from_chunks_and_dtype_unchecked(
        name: &str,
        chunks: Vec<ArrayRef>,
        dtype: &DataType,
    ) -> Self {
        use DataType::*;
        match dtype {
            #[cfg(feature = "dtype-i8")]
            Int8 => Int8Chunked::from_chunks(name, chunks).into_series(),
            #[cfg(feature = "dtype-i16")]
            Int16 => Int16Chunked::from_chunks(name, chunks).into_series(),
            Int32 => Int32Chunked::from_chunks(name, chunks).into_series(),
            Int64 => Int64Chunked::from_chunks(name, chunks).into_series(),
            #[cfg(feature = "dtype-u8")]
            UInt8 => UInt8Chunked::from_chunks(name, chunks).into_series(),
            #[cfg(feature = "dtype-u16")]
            UInt16 => UInt16Chunked::from_chunks(name, chunks).into_series(),
            UInt32 => UInt32Chunked::from_chunks(name, chunks).into_series(),
            UInt64 => UInt64Chunked::from_chunks(name, chunks).into_series(),
            #[cfg(feature = "dtype-date")]
            Date => Int32Chunked::from_chunks(name, chunks)
                .into_date()
                .into_series(),
            #[cfg(feature = "dtype-time")]
            Time => Int64Chunked::from_chunks(name, chunks)
                .into_time()
                .into_series(),
            #[cfg(feature = "dtype-duration")]
            Duration(tu) => Int64Chunked::from_chunks(name, chunks)
                .into_duration(*tu)
                .into_series(),
            #[cfg(feature = "dtype-datetime")]
            Datetime(tu, tz) => Int64Chunked::from_chunks(name, chunks)
                .into_datetime(*tu, tz.clone())
                .into_series(),
            List(_) => ListChunked::from_chunks(name, chunks).cast(dtype).unwrap(),
            Utf8 => Utf8Chunked::from_chunks(name, chunks).into_series(),
            #[cfg(feature = "dtype-categorical")]
            Categorical(rev_map) => {
                let cats = UInt32Chunked::from_chunks(name, chunks);
                CategoricalChunked::from_cats_and_rev_map(cats, rev_map.clone().unwrap())
                    .into_series()
            }
            Boolean => BooleanChunked::from_chunks(name, chunks).into_series(),
            Float32 => Float32Chunked::from_chunks(name, chunks).into_series(),
            Float64 => Float64Chunked::from_chunks(name, chunks).into_series(),
            #[cfg(feature = "dtype-struct")]
            Struct(_) => Series::try_from_arrow_unchecked(name, chunks, &dtype.to_arrow()).unwrap(),
            #[cfg(feature = "object")]
            Object(_) => todo!(),
            Null => panic!("null type not supported"),
            Unknown => panic!("uh oh, somehow we don't know the dtype?"),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }

    // Create a new Series without checking if the inner dtype of the chunks is correct
    // # Safety
    // The caller must ensure that the given `dtype` matches all the `ArrayRef` dtypes.
    pub(crate) unsafe fn try_from_arrow_unchecked(
        name: &str,
        chunks: Vec<ArrayRef>,
        dtype: &ArrowDataType,
    ) -> Result<Self> {
        match dtype {
            ArrowDataType::LargeUtf8 => Ok(Utf8Chunked::from_chunks(name, chunks).into_series()),
            ArrowDataType::Utf8 => {
                let chunks = cast_chunks(&chunks, &DataType::Utf8, false).unwrap();
                Ok(Utf8Chunked::from_chunks(name, chunks).into_series())
            }
            ArrowDataType::List(_) => {
                let chunks = chunks.iter().map(convert_inner_types).collect();
                Ok(ListChunked::from_chunks(name, chunks).into_series())
            }
            ArrowDataType::Boolean => Ok(BooleanChunked::from_chunks(name, chunks).into_series()),
            #[cfg(feature = "dtype-u8")]
            ArrowDataType::UInt8 => Ok(UInt8Chunked::from_chunks(name, chunks).into_series()),
            #[cfg(feature = "dtype-u16")]
            ArrowDataType::UInt16 => Ok(UInt16Chunked::from_chunks(name, chunks).into_series()),
            ArrowDataType::UInt32 => Ok(UInt32Chunked::from_chunks(name, chunks).into_series()),
            ArrowDataType::UInt64 => Ok(UInt64Chunked::from_chunks(name, chunks).into_series()),
            #[cfg(feature = "dtype-i8")]
            ArrowDataType::Int8 => Ok(Int8Chunked::from_chunks(name, chunks).into_series()),
            #[cfg(feature = "dtype-i16")]
            ArrowDataType::Int16 => Ok(Int16Chunked::from_chunks(name, chunks).into_series()),
            ArrowDataType::Int32 => Ok(Int32Chunked::from_chunks(name, chunks).into_series()),
            ArrowDataType::Int64 => Ok(Int64Chunked::from_chunks(name, chunks).into_series()),
            ArrowDataType::Float16 => {
                let chunks = cast_chunks(&chunks, &DataType::Float32, false).unwrap();
                Ok(Float32Chunked::from_chunks(name, chunks).into_series())
            }
            ArrowDataType::Float32 => Ok(Float32Chunked::from_chunks(name, chunks).into_series()),
            ArrowDataType::Float64 => Ok(Float64Chunked::from_chunks(name, chunks).into_series()),
            #[cfg(feature = "dtype-date")]
            ArrowDataType::Date32 => {
                let chunks = cast_chunks(&chunks, &DataType::Int32, false).unwrap();
                Ok(Int32Chunked::from_chunks(name, chunks)
                    .into_date()
                    .into_series())
            }
            #[cfg(feature = "dtype-datetime")]
            ArrowDataType::Date64 => {
                let chunks = cast_chunks(&chunks, &DataType::Int64, false).unwrap();
                let ca = Int64Chunked::from_chunks(name, chunks);
                Ok(ca.into_datetime(TimeUnit::Milliseconds, None).into_series())
            }
            #[cfg(feature = "dtype-datetime")]
            ArrowDataType::Timestamp(tu, tz) => {
                let mut tz = tz.clone();
                if tz.as_deref() == Some("") {
                    tz = None;
                }
                // we still drop timezone for now
                let chunks = cast_chunks(&chunks, &DataType::Int64, false).unwrap();
                let s = Int64Chunked::from_chunks(name, chunks)
                    .into_datetime(tu.into(), tz)
                    .into_series();
                Ok(match tu {
                    ArrowTimeUnit::Second => &s * MILLISECONDS,
                    ArrowTimeUnit::Millisecond => s,
                    ArrowTimeUnit::Microsecond => s,
                    ArrowTimeUnit::Nanosecond => s,
                })
            }
            #[cfg(feature = "dtype-duration")]
            ArrowDataType::Duration(tu) => {
                let chunks = cast_chunks(&chunks, &DataType::Int64, false).unwrap();
                let s = Int64Chunked::from_chunks(name, chunks)
                    .into_duration(tu.into())
                    .into_series();
                Ok(match tu {
                    ArrowTimeUnit::Second => &s * MILLISECONDS,
                    ArrowTimeUnit::Millisecond => s,
                    ArrowTimeUnit::Microsecond => s,
                    ArrowTimeUnit::Nanosecond => s,
                })
            }
            #[cfg(feature = "dtype-time")]
            ArrowDataType::Time64(tu) | ArrowDataType::Time32(tu) => {
                let mut chunks = chunks;
                if matches!(dtype, ArrowDataType::Time32(_)) {
                    chunks = cast_chunks(&chunks, &DataType::Int32, false).unwrap();
                }
                let chunks = cast_chunks(&chunks, &DataType::Int64, false).unwrap();
                let s = Int64Chunked::from_chunks(name, chunks)
                    .into_time()
                    .into_series();
                Ok(match tu {
                    ArrowTimeUnit::Second => &s * NANOSECONDS,
                    ArrowTimeUnit::Millisecond => &s * 1_000_000,
                    ArrowTimeUnit::Microsecond => &s * 1_000,
                    ArrowTimeUnit::Nanosecond => s,
                })
            }
            ArrowDataType::LargeList(_) => {
                let chunks = chunks.iter().map(convert_inner_types).collect();
                Ok(ListChunked::from_chunks(name, chunks).into_series())
            }
            ArrowDataType::Null => {
                // we don't support null types yet so we use a small digit type filled with nulls
                let len = chunks.iter().fold(0, |acc, array| acc + array.len());
                #[cfg(feature = "dtype-i8")]
                return Ok(Int8Chunked::full_null(name, len).into_series());
                #[cfg(not(feature = "dtype-i8"))]
                Ok(UInt32Chunked::full_null(name, len).into_series())
            }
            #[cfg(not(feature = "dtype-categorical"))]
            ArrowDataType::Dictionary(_, _, _) => {
                panic!("activate dtype-categorical to convert dictionary arrays")
            }
            #[cfg(feature = "dtype-categorical")]
            ArrowDataType::Dictionary(key_type, value_type, _) => {
                use arrow::datatypes::IntegerType;
                let chunks = chunks.iter().map(|arr| &**arr).collect::<Vec<_>>();
                let arr = arrow::compute::concatenate::concatenate(&chunks)?;

                if !matches!(
                    value_type.as_ref(),
                    ArrowDataType::Utf8 | ArrowDataType::LargeUtf8
                ) {
                    return Err(PolarsError::ComputeError(
                        "polars only support dictionaries with string like values".into(),
                    ));
                }

                macro_rules! unpack_keys_values {
                    ($dt:ty) => {{
                        let arr = arr.as_any().downcast_ref::<DictionaryArray<$dt>>().unwrap();
                        let keys = arr.keys();
                        let keys = cast(keys, &ArrowDataType::UInt32).unwrap();
                        let values = arr.values();
                        let values = cast(&**values, &ArrowDataType::LargeUtf8)?;
                        (keys, values)
                    }};
                }

                let (keys, values) = match key_type {
                    IntegerType::Int8 => {
                        unpack_keys_values!(i8)
                    }
                    IntegerType::UInt8 => {
                        unpack_keys_values!(u8)
                    }
                    IntegerType::Int16 => {
                        unpack_keys_values!(i16)
                    }
                    IntegerType::UInt16 => {
                        unpack_keys_values!(u16)
                    }
                    IntegerType::Int32 => {
                        unpack_keys_values!(i32)
                    }
                    IntegerType::UInt32 => {
                        unpack_keys_values!(u32)
                    }
                    _ => {
                        return Err(PolarsError::ComputeError(
                            "dictionaries with 64 bits keys are not supported by polars".into(),
                        ))
                    }
                };
                let keys = keys.as_any().downcast_ref::<PrimitiveArray<u32>>().unwrap();
                let values = values.as_any().downcast_ref::<Utf8Array<i64>>().unwrap();

                let mut builder = CategoricalChunkedBuilder::new(name, keys.len());
                let iter = keys
                    .into_iter()
                    .map(|opt_key| opt_key.map(|k| values.value_unchecked(*k as usize)));
                builder.drain_iter(iter);
                Ok(builder.finish().into_series())
            }
            #[cfg(not(feature = "dtype-u8"))]
            ArrowDataType::LargeBinary | ArrowDataType::Binary => {
                panic!("activate dtype-u8 to read binary data into polars List<u8>")
            }
            #[cfg(feature = "dtype-u8")]
            ArrowDataType::LargeBinary | ArrowDataType::Binary => {
                let chunks = chunks
                    .iter()
                    .map(|arr| {
                        let arr = cast(&**arr, &ArrowDataType::LargeBinary).unwrap();

                        let arr = arr.as_any().downcast_ref::<BinaryArray<i64>>().unwrap();
                        let values = arr.values().clone();
                        let offsets = arr.offsets().clone();
                        let validity = arr.validity().cloned();

                        let values = Box::new(PrimitiveArray::from_data(
                            ArrowDataType::UInt8,
                            values,
                            None,
                        ));

                        let dtype = ListArray::<i64>::default_datatype(ArrowDataType::UInt8);
                        // Safety:
                        // offsets are monotonically increasing
                        Box::new(ListArray::<i64>::new_unchecked(
                            dtype, offsets, values, validity,
                        )) as ArrayRef
                    })
                    .collect();
                Ok(ListChunked::from_chunks(name, chunks).into())
            }
            #[cfg(feature = "object")]
            ArrowDataType::Extension(s, _, Some(_)) if s == "POLARS_EXTENSION_TYPE" => {
                assert_eq!(chunks.len(), 1);
                let arr = chunks[0]
                    .as_any()
                    .downcast_ref::<FixedSizeBinaryArray>()
                    .unwrap();
                // Safety:
                // this is highly unsafe. it will dereference a raw ptr on the heap
                // make sure the ptr is allocated and from this pid
                // (the pid is checked before dereference)
                let s = {
                    let pe = PolarsExtension::new(arr.clone());
                    let s = pe.get_series(name);
                    pe.take_and_forget();
                    s
                };
                Ok(s)
            }
            #[cfg(feature = "dtype-struct")]
            ArrowDataType::Struct(_) => {
                let arr = if chunks.len() > 1 {
                    concatenate_owned_unchecked(&chunks).unwrap() as ArrayRef
                } else {
                    chunks[0].clone()
                };
                let arr = convert_inner_types(&arr);
                let mut struct_arr =
                    std::borrow::Cow::Borrowed(arr.as_any().downcast_ref::<StructArray>().unwrap());

                if let Some(validity) = struct_arr.validity() {
                    let new_values = struct_arr
                        .values()
                        .iter()
                        .map(|arr| match arr.validity() {
                            None => arr.with_validity(Some(validity.clone())),
                            Some(arr_validity) => arr.with_validity(Some(arr_validity & validity)),
                        })
                        .collect();

                    struct_arr = std::borrow::Cow::Owned(StructArray::new(
                        struct_arr.data_type().clone(),
                        new_values,
                        None,
                    ));
                }
                let fields = struct_arr
                    .values()
                    .iter()
                    .zip(struct_arr.fields())
                    .map(|(arr, field)| {
                        Series::try_from_arrow_unchecked(
                            &field.name,
                            vec![arr.clone()],
                            &field.data_type,
                        )
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(StructChunked::new_unchecked(name, &fields).into_series())
            }
            dt => Err(PolarsError::InvalidOperation(
                format!("Cannot create polars series from {:?} type", dt).into(),
            )),
        }
    }
}

fn convert_inner_types(arr: &ArrayRef) -> ArrayRef {
    match arr.data_type() {
        ArrowDataType::Utf8 => {
            let arr = arr.as_any().downcast_ref::<Utf8Array<i32>>().unwrap();
            Box::from(utf8_to_large_utf8(arr))
        }
        ArrowDataType::List(field) => {
            let out = cast(&**arr, &ArrowDataType::LargeList(field.clone())).unwrap();
            convert_inner_types(&out)
        }
        ArrowDataType::LargeList(_) => {
            let arr = arr.as_any().downcast_ref::<ListArray<i64>>().unwrap();
            let values = convert_inner_types(arr.values());
            let dtype = ListArray::<i64>::default_datatype(values.data_type().clone());
            unsafe {
                Box::from(ListArray::<i64>::new_unchecked(
                    dtype,
                    arr.offsets().clone(),
                    values,
                    arr.validity().cloned(),
                ))
            }
        }
        ArrowDataType::Struct(fields) => {
            let arr = arr.as_any().downcast_ref::<StructArray>().unwrap();
            let values = arr
                .values()
                .iter()
                .map(convert_inner_types)
                .collect::<Vec<_>>();

            let fields = fields
                .iter()
                .map(|f| ArrowField::new(&f.name, DataType::from(&f.data_type).to_arrow(), true))
                .collect();
            Box::new(StructArray::new(
                ArrowDataType::Struct(fields),
                values,
                arr.validity().cloned(),
            ))
        }
        _ => arr.clone(),
    }
}

// TODO: add types
impl TryFrom<(&str, Vec<ArrayRef>)> for Series {
    type Error = PolarsError;

    fn try_from(name_arr: (&str, Vec<ArrayRef>)) -> Result<Self> {
        let (name, chunks) = name_arr;

        let mut chunks_iter = chunks.iter();
        let data_type: ArrowDataType = chunks_iter
            .next()
            .ok_or_else(|| PolarsError::NoData("Expected at least on ArrayRef".into()))?
            .data_type()
            .clone();

        for chunk in chunks_iter {
            if chunk.data_type() != &data_type {
                return Err(PolarsError::InvalidOperation(
                    "Cannot create series from multiple arrays with different types".into(),
                ));
            }
        }
        // Safety:
        // dtype is checked
        unsafe { Series::try_from_arrow_unchecked(name, chunks, &data_type) }
    }
}

impl TryFrom<(&str, ArrayRef)> for Series {
    type Error = PolarsError;

    fn try_from(name_arr: (&str, ArrayRef)) -> Result<Self> {
        let (name, arr) = name_arr;
        Series::try_from((name, vec![arr]))
    }
}

pub trait IntoSeries {
    fn is_series() -> bool {
        false
    }

    fn into_series(self) -> Series
    where
        Self: Sized;
}

impl<T> From<ChunkedArray<T>> for Series
where
    T: PolarsDataType,
    ChunkedArray<T>: IntoSeries,
{
    fn from(ca: ChunkedArray<T>) -> Self {
        ca.into_series()
    }
}

#[cfg(feature = "dtype-date")]
impl From<DateChunked> for Series {
    fn from(a: DateChunked) -> Self {
        a.into_series()
    }
}

#[cfg(feature = "dtype-datetime")]
impl From<DatetimeChunked> for Series {
    fn from(a: DatetimeChunked) -> Self {
        a.into_series()
    }
}

#[cfg(feature = "dtype-duration")]
impl From<DurationChunked> for Series {
    fn from(a: DurationChunked) -> Self {
        a.into_series()
    }
}

#[cfg(feature = "dtype-time")]
impl From<TimeChunked> for Series {
    fn from(a: TimeChunked) -> Self {
        a.into_series()
    }
}

impl IntoSeries for Arc<dyn SeriesTrait> {
    fn into_series(self) -> Series {
        Series(self)
    }
}

impl IntoSeries for Series {
    fn is_series() -> bool {
        true
    }

    fn into_series(self) -> Series {
        self
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    #[cfg(feature = "dtype-u8")]
    fn test_binary_to_list() {
        let iter = std::iter::repeat(b"hello").take(2).map(Some);
        let a = Box::new(iter.collect::<BinaryArray<i32>>()) as ArrayRef;

        let s = Series::try_from(("", a)).unwrap();
        assert_eq!(s.dtype(), &DataType::List(Box::new(DataType::UInt8)));
    }
}
