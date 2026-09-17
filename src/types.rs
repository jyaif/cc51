//! C types for the MCS-51 target.

use std::rc::Rc;

/// Memory spaces of the 8051.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Space {
    /// Internal RAM, directly addressable (0x00-0x7f).
    Data,
    /// Internal RAM, indirectly addressable (0x00-0xff).
    Idata,
    /// External RAM (MOVX).
    Xdata,
    /// Paged external RAM.
    Pdata,
    /// Program memory.
    Code,
    /// Special function register (direct 0x80-0xff).
    Sfr,
    /// Bit-addressable location (bit address).
    Sbit,
    /// Bit variable in the bit-addressable RAM area.
    Bit,
}

impl Space {
    /// Generic pointer tag (SDCC compatible).
    pub fn gptr_tag(self) -> u8 {
        match self {
            Space::Xdata => 0x00,
            Space::Data | Space::Idata => 0x40,
            Space::Pdata => 0x60,
            Space::Code => 0x80,
            _ => 0x40,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Quals {
    pub is_const: bool,
    pub is_volatile: bool,
    pub space: Option<Space>,
}

pub type RecordId = usize;
pub type EnumId = usize;
/// Pointer address-space inference variable.
pub type SpaceVar = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntKind {
    Bool,
    /// SDCC __bit
    Bit,
    Char,
    Short,
    Int,
    Long,
    LongLong,
}

#[derive(Clone, Debug)]
pub enum TypeKind {
    Void,
    Int(IntKind, bool /* signed */),
    Float,
    Double,
    /// Pointer: pointee type and address-space variable.
    Pointer(Rc<Type>, SpaceVar),
    Array(Rc<Type>, Option<u32>),
    Func(Rc<FuncType>),
    Record(RecordId),
    Enum(EnumId, IntKind, bool),
}

#[derive(Clone, Debug)]
pub struct FuncType {
    pub ret: Type,
    pub params: Vec<Type>,
    pub variadic: bool,
    /// Declared with an empty parameter list `f()`.
    pub unprototyped: bool,
    pub attrs: FuncAttrs,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FuncAttrs {
    pub interrupt: Option<u8>,
    pub using: Option<u8>,
    pub naked: bool,
    pub critical: bool,
    pub reentrant: bool,
    pub noreturn: bool,
    pub preserves: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct Type {
    pub kind: TypeKind,
    pub q: Quals,
}

impl PartialEq for Type {
    fn eq(&self, o: &Type) -> bool {
        self.same(o)
    }
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: Option<Rc<str>>,
    pub ty: Type,
    pub offset: u32,
    /// Bit-field: (bit offset within the storage unit starting at `offset`, width).
    pub bits: Option<(u8, u8)>,
}

#[derive(Clone, Debug)]
pub struct RecordDef {
    pub tag: Option<Rc<str>>,
    pub is_union: bool,
    pub fields: Vec<Field>,
    pub size: u32,
    pub complete: bool,
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub tag: Option<Rc<str>>,
    pub complete: bool,
}

pub const PTR_SIZE: u32 = 2;

impl Type {
    pub fn new(kind: TypeKind) -> Type {
        Type { kind, q: Quals::default() }
    }
    pub fn void() -> Type {
        Type::new(TypeKind::Void)
    }
    pub fn int() -> Type {
        Type::new(TypeKind::Int(IntKind::Int, true))
    }
    pub fn uint() -> Type {
        Type::new(TypeKind::Int(IntKind::Int, false))
    }
    pub fn long() -> Type {
        Type::new(TypeKind::Int(IntKind::Long, true))
    }
    pub fn ulong() -> Type {
        Type::new(TypeKind::Int(IntKind::Long, false))
    }
    pub fn llong() -> Type {
        Type::new(TypeKind::Int(IntKind::LongLong, true))
    }
    pub fn ullong() -> Type {
        Type::new(TypeKind::Int(IntKind::LongLong, false))
    }
    pub fn char() -> Type {
        Type::new(TypeKind::Int(IntKind::Char, false))
    }
    pub fn schar() -> Type {
        Type::new(TypeKind::Int(IntKind::Char, true))
    }
    pub fn uchar() -> Type {
        Type::new(TypeKind::Int(IntKind::Char, false))
    }
    pub fn bool_() -> Type {
        Type::new(TypeKind::Int(IntKind::Bool, false))
    }
    pub fn bit() -> Type {
        Type::new(TypeKind::Int(IntKind::Bit, false))
    }
    pub fn intk(k: IntKind, signed: bool) -> Type {
        Type::new(TypeKind::Int(k, signed))
    }
    pub fn unqual(&self) -> Type {
        Type { kind: self.kind.clone(), q: Quals::default() }
    }
    pub fn with_quals(&self, q: Quals) -> Type {
        Type { kind: self.kind.clone(), q }
    }
    pub fn is_void(&self) -> bool {
        matches!(self.kind, TypeKind::Void)
    }
    pub fn is_integer(&self) -> bool {
        matches!(self.kind, TypeKind::Int(..) | TypeKind::Enum(..))
    }
    pub fn is_bool(&self) -> bool {
        matches!(self.kind, TypeKind::Int(IntKind::Bool | IntKind::Bit, _))
    }
    pub fn is_bit(&self) -> bool {
        matches!(self.kind, TypeKind::Int(IntKind::Bit, _))
    }
    pub fn is_float(&self) -> bool {
        matches!(self.kind, TypeKind::Float | TypeKind::Double)
    }
    pub fn is_arith(&self) -> bool {
        self.is_integer() || self.is_float()
    }
    pub fn is_pointer(&self) -> bool {
        matches!(self.kind, TypeKind::Pointer(..))
    }
    pub fn is_scalar(&self) -> bool {
        self.is_arith() || self.is_pointer()
    }
    pub fn is_array(&self) -> bool {
        matches!(self.kind, TypeKind::Array(..))
    }
    pub fn is_func(&self) -> bool {
        matches!(self.kind, TypeKind::Func(..))
    }
    pub fn is_record(&self) -> bool {
        matches!(self.kind, TypeKind::Record(..))
    }
    pub fn is_func_ptr(&self) -> bool {
        match &self.kind {
            TypeKind::Pointer(p, _) => p.is_func(),
            _ => false,
        }
    }
    pub fn is_signed(&self) -> bool {
        match self.kind {
            TypeKind::Int(_, s) => s,
            TypeKind::Enum(_, _, s) => s,
            TypeKind::Float | TypeKind::Double => true,
            _ => false,
        }
    }
    pub fn int_kind(&self) -> Option<(IntKind, bool)> {
        match self.kind {
            TypeKind::Int(k, s) => Some((k, s)),
            TypeKind::Enum(_, k, s) => Some((k, s)),
            _ => None,
        }
    }
    pub fn pointee(&self) -> Option<&Type> {
        match &self.kind {
            TypeKind::Pointer(p, _) => Some(p),
            TypeKind::Array(p, _) => Some(p),
            _ => None,
        }
    }
    pub fn space_var(&self) -> Option<SpaceVar> {
        match &self.kind {
            TypeKind::Pointer(_, v) => Some(*v),
            _ => None,
        }
    }
    pub fn func(&self) -> Option<&Rc<FuncType>> {
        match &self.kind {
            TypeKind::Func(f) => Some(f),
            TypeKind::Pointer(p, _) => match &p.kind {
                TypeKind::Func(f) => Some(f),
                _ => None,
            },
            _ => None,
        }
    }
    /// Integer rank for usual arithmetic conversions.
    pub fn rank(k: IntKind) -> u8 {
        match k {
            IntKind::Bool | IntKind::Bit => 0,
            IntKind::Char => 1,
            IntKind::Short => 2,
            IntKind::Int => 3,
            IntKind::Long => 4,
            IntKind::LongLong => 5,
        }
    }
    pub fn int_size(k: IntKind) -> u32 {
        match k {
            IntKind::Bool | IntKind::Bit | IntKind::Char => 1,
            IntKind::Short | IntKind::Int => 2,
            IntKind::Long => 4,
            IntKind::LongLong => 8,
        }
    }
    /// Structural type identity, ignoring qualifiers at the top level and space variables.
    pub fn same(&self, o: &Type) -> bool {
        match (&self.kind, &o.kind) {
            (TypeKind::Void, TypeKind::Void) => true,
            (TypeKind::Int(a, sa), TypeKind::Int(b, sb)) => a == b && sa == sb,
            (TypeKind::Float, TypeKind::Float) | (TypeKind::Double, TypeKind::Double) => true,
            (TypeKind::Pointer(a, _), TypeKind::Pointer(b, _)) => {
                a.same(b) && a.q.is_const == b.q.is_const && a.q.is_volatile == b.q.is_volatile
            }
            (TypeKind::Array(a, n), TypeKind::Array(b, m)) => a.same(b) && (n == m || n.is_none() || m.is_none()),
            (TypeKind::Func(a), TypeKind::Func(b)) => {
                a.ret.same(&b.ret)
                    && (a.unprototyped
                        || b.unprototyped
                        || (a.params.len() == b.params.len() && a.params.iter().zip(&b.params).all(|(x, y)| x.same(y))))
            }
            (TypeKind::Record(a), TypeKind::Record(b)) => a == b,
            (TypeKind::Enum(a, ..), TypeKind::Enum(b, ..)) => a == b,
            (TypeKind::Enum(_, k, s), TypeKind::Int(k2, s2)) | (TypeKind::Int(k2, s2), TypeKind::Enum(_, k, s)) => {
                k == k2 && s == s2
            }
            _ => false,
        }
    }
}

pub fn int_bits(k: IntKind) -> u32 {
    match k {
        IntKind::Bool | IntKind::Bit => 1,
        _ => Type::int_size(k) * 8,
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.q.is_const {
            write!(f, "const ")?;
        }
        if self.q.is_volatile {
            write!(f, "volatile ")?;
        }
        if let Some(s) = self.q.space {
            write!(f, "__{} ", format!("{:?}", s).to_lowercase())?;
        }
        match &self.kind {
            TypeKind::Void => write!(f, "void"),
            TypeKind::Int(k, s) => {
                let name = match k {
                    IntKind::Bool => return write!(f, "_Bool"),
                    IntKind::Bit => return write!(f, "__bit"),
                    IntKind::Char => "char",
                    IntKind::Short => "short",
                    IntKind::Int => "int",
                    IntKind::Long => "long",
                    IntKind::LongLong => "long long",
                };
                if *s {
                    if *k == IntKind::Char {
                        write!(f, "signed ")?;
                    }
                } else {
                    write!(f, "unsigned ")?;
                }
                write!(f, "{}", name)
            }
            TypeKind::Float => write!(f, "float"),
            TypeKind::Double => write!(f, "double"),
            TypeKind::Pointer(p, _) => write!(f, "{}*", p),
            TypeKind::Array(p, n) => match n {
                Some(n) => write!(f, "{}[{}]", p, n),
                None => write!(f, "{}[]", p),
            },
            TypeKind::Func(ft) => {
                write!(f, "{}(", ft.ret)?;
                for (i, p) in ft.params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                if ft.variadic {
                    write!(f, ", ...")?;
                }
                write!(f, ")")
            }
            TypeKind::Record(id) => write!(f, "struct#{}", id),
            TypeKind::Enum(id, ..) => write!(f, "enum#{}", id),
        }
    }
}
