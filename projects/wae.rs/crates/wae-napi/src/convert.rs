//! JS ↔ Rust 值转换。
//!
//! 这里是「签名即契约」的落地层：导出的 Rust 函数直接写领域类型，
//! 参数与返回值的转换、类型错误文本以及 TypeScript 声明都由同一组 trait 决定。
//! 转换失败必须说清「期望什么、实际是什么」，不得回落 `napi status N`。

use std::collections::HashMap;

use crate::{Env, Error, JsValue, Result};

/// 从 JS 值构造 Rust 值。
pub trait FromJs: Sized {
    /// 转换一个 JS 值；失败时错误文本必须包含期望类型。
    fn from_js(value: JsValue) -> Result<Self>;
}

/// 把 Rust 值写入 JS。
pub trait ToJs {
    /// 转换并写入当前环境。
    fn to_js(self, env: Env) -> Result<JsValue>;
}

/// 任意 serde 类型的 JSON 直通包装。
///
/// 用于领域类型尚未提供 `FromJs` / `ToJs` 的场合；边界上按 JSON 结构转换，
/// 不做字段级强类型检查。优先为新导出写 `#[object]`，只在迁移期或自由形状数据上用它。
#[derive(Debug, Clone, PartialEq)]
pub struct Json<T>(pub T);

fn mismatch(expected: &str, value: JsValue) -> Error {
    Error::new(format!("expected {expected}, got {}", value.describe()))
}

impl FromJs for JsValue {
    fn from_js(value: JsValue) -> Result<Self> {
        Ok(value)
    }
}

impl ToJs for JsValue {
    fn to_js(self, _env: Env) -> Result<JsValue> {
        Ok(self)
    }
}

impl FromJs for () {
    fn from_js(_value: JsValue) -> Result<Self> {
        Ok(())
    }
}

impl ToJs for () {
    fn to_js(self, env: Env) -> Result<JsValue> {
        env.undefined()
    }
}

impl FromJs for bool {
    fn from_js(value: JsValue) -> Result<Self> {
        if value.type_of()? != crate::sys::napi_boolean {
            return Err(mismatch("boolean", value));
        }
        value.as_bool()
    }
}

impl ToJs for bool {
    fn to_js(self, env: Env) -> Result<JsValue> {
        env.boolean(self)
    }
}

impl FromJs for String {
    fn from_js(value: JsValue) -> Result<Self> {
        if value.type_of()? != crate::sys::napi_string {
            return Err(mismatch("string", value));
        }
        value.as_string()
    }
}

impl ToJs for String {
    fn to_js(self, env: Env) -> Result<JsValue> {
        env.string(&self)
    }
}

impl ToJs for &str {
    fn to_js(self, env: Env) -> Result<JsValue> {
        env.string(self)
    }
}

impl FromJs for f64 {
    fn from_js(value: JsValue) -> Result<Self> {
        if value.type_of()? != crate::sys::napi_number {
            return Err(mismatch("number", value));
        }
        value.as_f64()
    }
}

impl FromJs for f32 {
    fn from_js(value: JsValue) -> Result<Self> {
        Ok(f64::from_js(value)? as f32)
    }
}

impl ToJs for f64 {
    fn to_js(self, env: Env) -> Result<JsValue> {
        env.double(self)
    }
}

impl ToJs for f32 {
    fn to_js(self, env: Env) -> Result<JsValue> {
        env.double(f64::from(self))
    }
}

macro_rules! int_from_js {
    ($($ty:ty),* $(,)?) => {
        $(
            impl FromJs for $ty {
                fn from_js(value: JsValue) -> Result<Self> {
                    if value.type_of()? != crate::sys::napi_number {
                        return Err(mismatch("integer", value));
                    }
                    let raw = value.as_i64()?;
                    <$ty>::try_from(raw).map_err(|_| {
                        Error::new(format!("expected {} in range, got {}", stringify!($ty), raw))
                    })
                }
            }

            impl ToJs for $ty {
                fn to_js(self, env: Env) -> Result<JsValue> {
                    env.int64(self as i64)
                }
            }
        )*
    };
}

int_from_js!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl<T: FromJs> FromJs for Option<T> {
    fn from_js(value: JsValue) -> Result<Self> {
        if value.is_undefined()? || value.type_of()? == crate::sys::napi_null {
            return Ok(None);
        }
        T::from_js(value).map(Some)
    }
}

impl<T: ToJs> ToJs for Option<T> {
    fn to_js(self, env: Env) -> Result<JsValue> {
        match self {
            Some(value) => value.to_js(env),
            None => env.undefined(),
        }
    }
}

impl<T: FromJs> FromJs for Vec<T> {
    fn from_js(value: JsValue) -> Result<Self> {
        if !value.is_array()? {
            return Err(mismatch("array", value));
        }
        let length = value.get_named("length")?.as_i64()?.max(0);
        let mut items = Vec::with_capacity(length as usize);
        for index in 0..length as u32 {
            items.push(T::from_js(value.element(index)?)?);
        }
        Ok(items)
    }
}

impl<T: ToJs> ToJs for Vec<T> {
    fn to_js(self, env: Env) -> Result<JsValue> {
        let array = env.array_with_length(self.len())?;
        for (index, item) in self.into_iter().enumerate() {
            array.set_element(index as u32, item.to_js(env)?)?;
        }
        Ok(array)
    }
}

impl<T: FromJs> FromJs for HashMap<String, T> {
    fn from_js(value: JsValue) -> Result<Self> {
        if value.type_of()? != crate::sys::napi_object || value.is_array()? {
            return Err(mismatch("object", value));
        }
        let names = value.property_names()?;
        let length = names.get_named("length")?.as_i64()?.max(0);
        let mut map = HashMap::with_capacity(length as usize);
        for index in 0..length as u32 {
            let key = names.element(index)?.as_string()?;
            map.insert(key.clone(), T::from_js(value.get_named(&key)?)?);
        }
        Ok(map)
    }
}

impl<T: ToJs> ToJs for HashMap<String, T> {
    fn to_js(self, env: Env) -> Result<JsValue> {
        let object = env.object()?;
        for (key, item) in self {
            object.set_named(&key, item.to_js(env)?)?;
        }
        Ok(object)
    }
}

impl FromJs for serde_json::Value {
    fn from_js(value: JsValue) -> Result<Self> {
        js_to_json(value)
    }
}

impl ToJs for serde_json::Value {
    fn to_js(self, env: Env) -> Result<JsValue> {
        json_to_js(env, &self)
    }
}

impl<T> FromJs for Json<T>
where
    T: serde::de::DeserializeOwned,
{
    fn from_js(value: JsValue) -> Result<Self> {
        let json = js_to_json(value)?;
        serde_json::from_value(json).map(Json).map_err(|error| Error::new(format!("expected JSON value: {error}")))
    }
}

impl<T> ToJs for Json<T>
where
    T: serde::Serialize,
{
    fn to_js(self, env: Env) -> Result<JsValue> {
        let json = serde_json::to_value(self.0).map_err(|error| Error::new(format!("JSON serialization failed: {error}")))?;
        json_to_js(env, &json)
    }
}

/// 把 `serde_json::Value` 写入 JS，保持对象与数组结构。
pub fn json_to_js(env: Env, value: &serde_json::Value) -> Result<JsValue> {
    match value {
        serde_json::Value::Null => env.null(),
        serde_json::Value::Bool(inner) => env.boolean(*inner),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(inner) => env.int64(inner),
            None => env.double(number.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::String(text) => env.string(text),
        serde_json::Value::Array(items) => {
            let array = env.array_with_length(items.len())?;
            for (index, item) in items.iter().enumerate() {
                array.set_element(index as u32, json_to_js(env, item)?)?;
            }
            Ok(array)
        }
        serde_json::Value::Object(map) => {
            let object = env.object()?;
            for (key, item) in map {
                object.set_named(key, json_to_js(env, item)?)?;
            }
            Ok(object)
        }
    }
}

/// 把 JS 值读成 `serde_json::Value`；函数、符号等不可表示的值显式失败。
pub fn js_to_json(value: JsValue) -> Result<serde_json::Value> {
    use crate::sys::{napi_boolean, napi_null, napi_number, napi_object, napi_string};

    let kind = value.type_of()?;
    if kind == napi_null || value.is_undefined()? {
        return Ok(serde_json::Value::Null);
    }
    if kind == napi_boolean {
        return Ok(serde_json::Value::Bool(value.as_bool()?));
    }
    if kind == napi_number {
        let number = value.as_f64()?;
        if let Ok(integer) = value.as_i64()
            && number == integer as f64
        {
            return Ok(serde_json::Value::Number(integer.into()));
        }
        return serde_json::Number::from_f64(number).map(serde_json::Value::Number).ok_or_else(|| Error::new("expected a finite number"));
    }
    if kind == napi_string {
        return Ok(serde_json::Value::String(value.as_string()?));
    }
    if kind == napi_object {
        if value.is_array()? {
            let length = value.get_named("length")?.as_i64()?.max(0);
            let mut items = Vec::with_capacity(length as usize);
            for index in 0..length as u32 {
                items.push(js_to_json(value.element(index)?)?);
            }
            return Ok(serde_json::Value::Array(items));
        }
        let names = value.property_names()?;
        let length = names.get_named("length")?.as_i64()?.max(0);
        let mut map = serde_json::Map::with_capacity(length as usize);
        for index in 0..length as u32 {
            let key = names.element(index)?.as_string()?;
            map.insert(key.clone(), js_to_json(value.get_named(&key)?)?);
        }
        return Ok(serde_json::Value::Object(map));
    }
    Err(Error::new(format!("expected a JSON value, got {}", value.describe())))
}

impl<T: ToJs> ToJs for &T
where
    T: Clone + ToJs,
{
    fn to_js(self, env: Env) -> Result<JsValue> {
        self.clone().to_js(env)
    }
}
