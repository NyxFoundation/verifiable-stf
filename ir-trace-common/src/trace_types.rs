use serde::{Deserialize, Serialize};

use crate::value::Value;

pub type ValueId = u32;

pub const TRACE_MAGIC: [u8; 4] = *b"LT02";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceHeader {
    pub magic: [u8; 4],
    pub ir_program_hash: [u8; 32],
    pub input_hash: [u8; 32],
    pub output_hash: [u8; 32],
    pub value_count: u32,
    pub step_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrimOp {
    // UInt arithmetic
    NatAdd,
    NatSub,
    NatMul,
    NatDiv,
    NatMod,
    NatBeq,
    NatBlt,
    NatBle,
    NatDecEq,
    NatLand,
    NatLor,
    NatXor,
    NatShiftLeft,
    NatShiftRight,
    // UInt8/16/32/64 ops
    UInt8Add,
    UInt8Sub,
    UInt8Mul,
    UInt8Div,
    UInt8Mod,
    UInt8Land,
    UInt8Lor,
    UInt8Xor,
    UInt8ShiftLeft,
    UInt8ShiftRight,
    UInt8DecEq,
    UInt8Beq,
    UInt8Blt,
    UInt8Ble,
    UInt16Add,
    UInt16Sub,
    UInt16Mul,
    UInt16Div,
    UInt16Mod,
    UInt16Land,
    UInt16Lor,
    UInt16Xor,
    UInt16ShiftLeft,
    UInt16ShiftRight,
    UInt16DecEq,
    UInt16Beq,
    UInt16Blt,
    UInt16Ble,
    UInt32Add,
    UInt32Sub,
    UInt32Mul,
    UInt32Div,
    UInt32Mod,
    UInt32Land,
    UInt32Lor,
    UInt32Xor,
    UInt32ShiftLeft,
    UInt32ShiftRight,
    UInt32DecEq,
    UInt32Beq,
    UInt32Blt,
    UInt32Ble,
    UInt64Add,
    UInt64Sub,
    UInt64Mul,
    UInt64Div,
    UInt64Mod,
    UInt64Land,
    UInt64Lor,
    UInt64Xor,
    UInt64ShiftLeft,
    UInt64ShiftRight,
    UInt64DecEq,
    UInt64Beq,
    UInt64Blt,
    UInt64Ble,
    // Array operations
    ArrayMkEmpty,
    ArraySize,
    ArrayGet,
    ArraySet,
    ArrayPush,
    // ByteArray operations
    ByteArrayMkEmpty,
    ByteArraySize,
    ByteArrayGet,
    ByteArraySet,
    ByteArrayPush,
    ByteArrayAppend,
    ByteArrayCopySlice,
    ByteArrayEmptyWithCapacity,
    // String operations
    StringLength,
    StringAppend,
    StringDecEq,
    StringMk,
    // Conversion
    UInt32ToNat,
    UInt64ToNat,
    NatToUInt32,
    NatToUInt64,
    UInt8ToNat,
    UInt16ToNat,
    NatToUInt8,
    NatToUInt16,
    StringToNat,
    // Bool
    BoolNot,
    // Other
    Panic,
    DbgTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TraceStep {
    Branch {
        scrutinee: ValueId,
        chosen_tag: u16,
    },
    PrimResult {
        op: PrimOp,
        args: Vec<ValueId>,
        result: ValueId,
    },
    CtorCreate {
        tag: u16,
        fields: Vec<ValueId>,
        scalar_data: Vec<u8>,
        result: ValueId,
    },
    ProjResult {
        obj: ValueId,
        idx: u16,
        result: ValueId,
    },
    SetResult {
        obj: ValueId,
        idx: u16,
        val: ValueId,
        result: ValueId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub header: TraceHeader,
    pub value_table: Vec<Value>,
    pub steps: Vec<TraceStep>,
    pub output_value_id: ValueId,
}
