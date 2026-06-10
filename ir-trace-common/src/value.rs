use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Value {
    Scalar(u64),
    Object {
        tag: u16,
        fields: Vec<Value>,
        scalars: Vec<u8>,
    },
    Array(Vec<Value>),
    ByteArray(Vec<u8>),
    Closure {
        fn_id: String,
        arity: u32,
        captured: Vec<Value>,
    },
    Nat(Vec<u8>), // big-endian bytes for arbitrary-precision nat
    Str(String),
    Irrelevant,
}

impl Value {
    pub fn tag(&self) -> u16 {
        match self {
            Value::Object { tag, .. } => *tag,
            Value::Scalar(v) => *v as u16,
            _ => 0,
        }
    }

    pub fn field(&self, idx: usize) -> &Value {
        match self {
            Value::Object { fields, .. } => {
                fields.get(idx).unwrap_or(&Value::Irrelevant)
            }
            // Scalars, Irrelevant, etc. - return Irrelevant for field access
            // This happens when a boxed value is projected
            _ => &Value::Irrelevant,
        }
    }

    pub fn set_field(&mut self, idx: usize, val: Value) {
        match self {
            Value::Object { fields, .. } => {
                if idx < fields.len() {
                    fields[idx] = val;
                }
            }
            _ => {}
        }
    }

    pub fn get_scalar_bytes(&self, _n: u32, offset: u32, size: usize) -> Vec<u8> {
        match self {
            Value::Object { scalars, .. } => {
                let start = offset as usize;
                let end = (start + size).min(scalars.len());
                if start < scalars.len() {
                    let mut result = scalars[start..end].to_vec();
                    result.resize(size, 0);
                    result
                } else {
                    vec![0u8; size]
                }
            }
            Value::Scalar(v) => {
                let bytes = v.to_le_bytes();
                let start = offset as usize;
                let mut result = vec![0u8; size];
                for i in 0..size {
                    if start + i < 8 {
                        result[i] = bytes[start + i];
                    }
                }
                result
            }
            _ => vec![0u8; size],
        }
    }

    pub fn set_scalar_bytes(&mut self, _n: u32, offset: u32, data: &[u8]) {
        match self {
            Value::Object { scalars, .. } => {
                let start = offset as usize;
                let end = start + data.len();
                if scalars.len() < end {
                    scalars.resize(end, 0);
                }
                scalars[start..end].copy_from_slice(data);
            }
            _ => {}
        }
    }

    pub fn as_u64(&self) -> u64 {
        match self {
            Value::Scalar(v) => *v,
            Value::Irrelevant => 0,
            Value::Object { tag, .. } => *tag as u64,
            _ => panic!("as_u64 on non-scalar: {:?}", self),
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Value::Scalar(v) => *v != 0,
            Value::Object { tag, .. } => *tag != 0,
            _ => false,
        }
    }

    pub fn serialize_to_bytes(&self) -> Vec<u8> {
        // Simple serialization for output commitment
        match self {
            Value::ByteArray(data) => data.clone(),
            Value::Scalar(v) => v.to_le_bytes().to_vec(),
            _ => {
                let json = serde_json::to_vec(self).unwrap_or_default();
                json
            }
        }
    }
}
