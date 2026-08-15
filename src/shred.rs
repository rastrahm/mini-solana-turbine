//! Layout binario de shreds (data vs code) y parseo zero-copy.
//!
//! El payload es un subslice del buffer de entrada (slot de arena). Los headers
//! se leen como `#[repr(C, packed)]` ([`ShredHeader`]): align 1, acceso a campos solo
//! por valor (nunca `&header.slot`).

use crate::Error;
use core::fmt;
use core::mem::{align_of, size_of};
use core::ptr;

/// Tipo Solana-like de shred de datos (legacy data).
pub const SHRED_TYPE_DATA: u8 = 0xA5;
/// Tipo Solana-like de shred de paridad (legacy code).
pub const SHRED_TYPE_CODE: u8 = 0x5A;

/// Bytes del header común en el wire (sin padding de rustc: el tipo está packed).
pub const COMMON_HEADER_SIZE: usize = size_of::<ShredHeader>();
/// Bytes del subheader data.
pub const DATA_HEADER_SIZE: usize = size_of::<DataShredHeader>();
/// Bytes del subheader code.
pub const CODE_HEADER_SIZE: usize = size_of::<CodeShredHeader>();
/// Mínimo de un data shred (headers; payload puede ser vacío).
pub const DATA_SHRED_OVERHEAD: usize = COMMON_HEADER_SIZE + DATA_HEADER_SIZE;
/// Mínimo de un code shred.
pub const CODE_SHRED_OVERHEAD: usize = COMMON_HEADER_SIZE + CODE_HEADER_SIZE;

/// Header común. Packed para que `size_of` coincida con el wire (20 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ShredHeader {
    slot: u64,
    fec_set_index: u32,
    index: u32,
    version: u16,
    shred_type: u8,
    reserved: u8,
}

/// Subheader de un data shred (4 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct DataShredHeader {
    parent_offset: u16,
    flags: u8,
    reserved: u8,
}

/// Subheader de un code shred (6 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct CodeShredHeader {
    num_data: u16,
    num_code: u16,
    position: u16,
}

/// Vista zero-copy de un shred ya validado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shred<'a> {
    /// Shred de ledger (payload = entradas/bytes del bloque).
    Data(DataShred<'a>),
    /// Shred de paridad Reed-Solomon.
    Code(CodeShred<'a>),
}

/// Data shred: headers copiados + payload prestado del slot.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DataShred<'a> {
    header: ShredHeader,
    data: DataShredHeader,
    payload: &'a [u8],
}

/// Code shred: headers copiados + payload prestado del slot.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CodeShred<'a> {
    header: ShredHeader,
    code: CodeShredHeader,
    payload: &'a [u8],
}

impl ShredHeader {
    /// Purpose: Header de data shred (`shred_type = DATA`, `reserved = 0`).
    /// Inputs: `slot` — slot de ledger; `fec_set_index` — primer índice del set FEC;
    ///   `index` — índice del shred en el slot; `version` — versión de shreds.
    /// Returns: Header packed listo para `encode_data`.
    #[inline(always)]
    pub const fn data(slot: u64, fec_set_index: u32, index: u32, version: u16) -> Self {
        Self {
            slot,
            fec_set_index,
            index,
            version,
            shred_type: SHRED_TYPE_DATA,
            reserved: 0,
        }
    }

    /// Purpose: Header de code shred (`shred_type = CODE`, `reserved = 0`).
    /// Inputs: mismos campos que [`data`](Self::data).
    /// Returns: Header packed listo para `encode_code`.
    #[inline(always)]
    pub const fn code(slot: u64, fec_set_index: u32, index: u32, version: u16) -> Self {
        Self {
            slot,
            fec_set_index,
            index,
            version,
            shred_type: SHRED_TYPE_CODE,
            reserved: 0,
        }
    }

    /// Purpose: Slot de ledger (copia; no forma `&u64` packed).
    /// Inputs: `self` — header alineado en stack o leído unaligned.
    /// Returns: `u64` little-endian del wire.
    #[inline(always)]
    pub const fn slot(&self) -> u64 {
        self.slot
    }

    /// Purpose: Índice del primer shred del set FEC.
    /// Inputs: `self`.
    /// Returns: `u32`.
    #[inline(always)]
    pub const fn fec_set_index(&self) -> u32 {
        self.fec_set_index
    }

    /// Purpose: Índice de este shred dentro del slot.
    /// Inputs: `self`.
    /// Returns: `u32`; debe ser `>= fec_set_index`.
    #[inline(always)]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Purpose: Versión de protocolo de shreds.
    /// Inputs: `self`.
    /// Returns: `u16`.
    #[inline(always)]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Purpose: Discriminante data vs code.
    /// Inputs: `self`.
    /// Returns: [`SHRED_TYPE_DATA`] o [`SHRED_TYPE_CODE`] si el paquete es válido.
    #[inline(always)]
    pub const fn shred_type(&self) -> u8 {
        self.shred_type
    }

    /// Purpose: Byte reservado del common header.
    /// Inputs: `self`.
    /// Returns: normalmente 0.
    #[inline(always)]
    pub const fn reserved(&self) -> u8 {
        self.reserved
    }
}

impl DataShredHeader {
    /// Purpose: Subheader data.
    /// Inputs: `parent_offset` — `slot - parent_slot`; `flags` — bits de complete/last.
    /// Returns: Subheader packed (`reserved = 0`).
    #[inline(always)]
    pub const fn new(parent_offset: u16, flags: u8) -> Self {
        Self {
            parent_offset,
            flags,
            reserved: 0,
        }
    }

    /// Purpose: Offset al slot padre.
    /// Inputs: `self`.
    /// Returns: `u16`.
    #[inline(always)]
    pub const fn parent_offset(&self) -> u16 {
        self.parent_offset
    }

    /// Purpose: Flags de data shred.
    /// Inputs: `self`.
    /// Returns: `u8` (el crate aún no interpreta bits individuales).
    #[inline(always)]
    pub const fn flags(&self) -> u8 {
        self.flags
    }
}

impl CodeShredHeader {
    /// Purpose: Subheader de paridad.
    /// Inputs: `num_data` — shreds de datos del set; `num_code` — shreds de paridad;
    ///   `position` — índice de este code shred en `[0, num_code)`.
    /// Returns: Subheader packed.
    #[inline(always)]
    pub const fn new(num_data: u16, num_code: u16, position: u16) -> Self {
        Self {
            num_data,
            num_code,
            position,
        }
    }

    /// Purpose: Número de data shreds del set FEC.
    /// Inputs: `self`.
    /// Returns: `u16` ≥ 1 si el shred es válido.
    #[inline(always)]
    pub const fn num_data(&self) -> u16 {
        self.num_data
    }

    /// Purpose: Número de code shreds del set FEC.
    /// Inputs: `self`.
    /// Returns: `u16` ≥ 1 si el shred es válido.
    #[inline(always)]
    pub const fn num_code(&self) -> u16 {
        self.num_code
    }

    /// Purpose: Posición de este code shred en el set.
    /// Inputs: `self`.
    /// Returns: `u16`; debe ser `< num_code`.
    #[inline(always)]
    pub const fn position(&self) -> u16 {
        self.position
    }
}

impl<'a> DataShred<'a> {
    /// Purpose: Header común copiado.
    /// Inputs: `self`.
    /// Returns: [`ShredHeader`] by value.
    #[inline(always)]
    pub const fn header(&self) -> ShredHeader {
        self.header
    }

    /// Purpose: Subheader data copiado.
    /// Inputs: `self`.
    /// Returns: [`DataShredHeader`].
    #[inline(always)]
    pub const fn data_header(&self) -> DataShredHeader {
        self.data
    }

    /// Purpose: Payload sin copiar.
    /// Inputs: `self`.
    /// Returns: Subslice del buffer original, después de los headers.
    #[inline(always)]
    pub const fn payload(&self) -> &'a [u8] {
        self.payload
    }
}

impl<'a> CodeShred<'a> {
    /// Purpose: Header común copiado.
    /// Inputs: `self`.
    /// Returns: [`ShredHeader`].
    #[inline(always)]
    pub const fn header(&self) -> ShredHeader {
        self.header
    }

    /// Purpose: Subheader code copiado.
    /// Inputs: `self`.
    /// Returns: [`CodeShredHeader`].
    #[inline(always)]
    pub const fn code_header(&self) -> CodeShredHeader {
        self.code
    }

    /// Purpose: Payload de paridad sin copiar.
    /// Inputs: `self`.
    /// Returns: Subslice del buffer original.
    #[inline(always)]
    pub const fn payload(&self) -> &'a [u8] {
        self.payload
    }
}

impl<'a> Shred<'a> {
    /// Purpose: Header común de cualquiera de las variantes.
    /// Inputs: `self`.
    /// Returns: [`ShredHeader`] copiado.
    #[inline(always)]
    pub const fn header(&self) -> ShredHeader {
        match *self {
            Shred::Data(s) => s.header,
            Shred::Code(s) => s.header,
        }
    }

    /// Purpose: Payload zero-copy.
    /// Inputs: `self`.
    /// Returns: Bytes tras los headers.
    #[inline(always)]
    pub const fn payload(&self) -> &'a [u8] {
        match *self {
            Shred::Data(s) => s.payload,
            Shred::Code(s) => s.payload,
        }
    }
}

/// Purpose: Interpreta bytes de un slot como data o code shred.
/// Inputs: `bytes` — `arena.slot(id)`, longitud comprometida (no la capacidad).
/// Returns: Vista con payload prestado; `ShredTruncated` / `ShredInvalidType` / `ShredInvalidFec`.
#[inline(always)]
pub fn parse(bytes: &[u8]) -> Result<Shred<'_>, Error> {
    let header = read_common(bytes)?;
    match header.shred_type() {
        SHRED_TYPE_DATA => parse_data(header, bytes),
        SHRED_TYPE_CODE => parse_code(header, bytes),
        _ => Err(Error::ShredInvalidType),
    }
}

/// Purpose: Valida longitud/alineación y proyecta el header común.
/// Inputs: `bytes` — prefijo del paquete.
/// Returns: Copia unaligned del header o `ShredTruncated`.
#[inline(always)]
fn read_common(bytes: &[u8]) -> Result<ShredHeader, Error> {
    let header = unsafe { read_packed::<ShredHeader>(bytes)? };
    Ok(header)
}

/// Purpose: Completa un data shred tras haber leído el common header.
/// Inputs: `header` — ya con tipo DATA; `bytes` — paquete completo.
/// Returns: [`Shred::Data`] o error de truncado / FEC.
#[inline(always)]
fn parse_data<'a>(header: ShredHeader, bytes: &'a [u8]) -> Result<Shred<'a>, Error> {
    if bytes.len() < DATA_SHRED_OVERHEAD {
        return Err(Error::ShredTruncated);
    }
    let data = unsafe { read_packed::<DataShredHeader>(&bytes[COMMON_HEADER_SIZE..])? };
    validate_data_fec(&header)?;
    Ok(Shred::Data(DataShred {
        header,
        data,
        payload: &bytes[DATA_SHRED_OVERHEAD..],
    }))
}

/// Purpose: Completa un code shred tras haber leído el common header.
/// Inputs: `header` — ya con tipo CODE; `bytes` — paquete completo.
/// Returns: [`Shred::Code`] o error de truncado / FEC.
#[inline(always)]
fn parse_code<'a>(header: ShredHeader, bytes: &'a [u8]) -> Result<Shred<'a>, Error> {
    if bytes.len() < CODE_SHRED_OVERHEAD {
        return Err(Error::ShredTruncated);
    }
    let code = unsafe { read_packed::<CodeShredHeader>(&bytes[COMMON_HEADER_SIZE..])? };
    validate_code_fec(&header, &code)?;
    Ok(Shred::Code(CodeShred {
        header,
        code,
        payload: &bytes[CODE_SHRED_OVERHEAD..],
    }))
}

/// Purpose: Lee un POD packed desde `bytes` tras chequear tamaño y align.
/// Inputs: `bytes` — debe cubrir `size_of::<T>()`; `T` es `repr(C, packed)`.
/// Returns: Copia de `T`; `ShredTruncated` si falta longitud o la alineación falla.
///
/// # Safety
///
/// El llamador garantiza que `T` es un header packed de este módulo (sin drop,
/// sin padding interpretado). La función vuelve a comprobar len y `align_offset`.
#[inline(always)]
unsafe fn read_packed<T: Copy>(bytes: &[u8]) -> Result<T, Error> {
    if bytes.len() < size_of::<T>() {
        return Err(Error::ShredTruncated);
    }
    let ptr = bytes.as_ptr();
    if ptr.align_offset(align_of::<T>()) != 0 {
        return Err(Error::ShredTruncated);
    }
    // SAFETY: len >= size_of::<T>(), align_of::<T>() es 1 (packed) o se comprobó
    // align_offset == 0, T es POD Copy, solo lectura, no se crean refs a campos.
    Ok(unsafe { ptr::read_unaligned(ptr.cast::<T>()) })
}

/// Purpose: `index >= fec_set_index` en data shreds.
/// Inputs: `header` — common header tipo DATA.
/// Returns: `Ok(())` o `ShredInvalidFec`.
#[inline(always)]
fn validate_data_fec(header: &ShredHeader) -> Result<(), Error> {
    if header.index() < header.fec_set_index() {
        return Err(Error::ShredInvalidFec);
    }
    Ok(())
}

/// Purpose: Conjunto FEC de un code shred: tamaños no nulos y posición in-range.
/// Inputs: `header` — common; `code` — subheader de paridad.
/// Returns: `Ok(())` o `ShredInvalidFec`.
#[inline(always)]
fn validate_code_fec(header: &ShredHeader, code: &CodeShredHeader) -> Result<(), Error> {
    if header.index() < header.fec_set_index() {
        return Err(Error::ShredInvalidFec);
    }
    if code.num_data() == 0 || code.num_code() == 0 {
        return Err(Error::ShredInvalidFec);
    }
    if code.position() >= code.num_code() {
        return Err(Error::ShredInvalidFec);
    }
    Ok(())
}

/// Purpose: Serializa un data shred sobre `dest` (slot_mut) sin heap.
/// Inputs: `dest` — capacidad del slot; `header` — se fuerza tipo DATA;
///   `data` — subheader; `payload` — bytes a proyectar tras los headers.
/// Returns: bytes escritos (overhead + payload) o truncado / FEC inválido.
pub fn encode_data(
    dest: &mut [u8],
    mut header: ShredHeader,
    data: DataShredHeader,
    payload: &[u8],
) -> Result<usize, Error> {
    header.shred_type = SHRED_TYPE_DATA;
    validate_data_fec(&header)?;
    let n = DATA_SHRED_OVERHEAD + payload.len();
    if dest.len() < n {
        return Err(Error::ShredTruncated);
    }
    unsafe {
        write_packed(&mut dest[..COMMON_HEADER_SIZE], header)?;
        write_packed(&mut dest[COMMON_HEADER_SIZE..DATA_SHRED_OVERHEAD], data)?;
    }
    dest[DATA_SHRED_OVERHEAD..n].copy_from_slice(payload);
    Ok(n)
}

/// Purpose: Serializa un code shred sobre `dest` sin heap.
/// Inputs: `dest` — slot; `header` — se fuerza tipo CODE; `code` — FEC;
///   `payload` — símbolos de paridad.
/// Returns: bytes escritos o truncado / FEC inválido.
pub fn encode_code(
    dest: &mut [u8],
    mut header: ShredHeader,
    code: CodeShredHeader,
    payload: &[u8],
) -> Result<usize, Error> {
    header.shred_type = SHRED_TYPE_CODE;
    validate_code_fec(&header, &code)?;
    let n = CODE_SHRED_OVERHEAD + payload.len();
    if dest.len() < n {
        return Err(Error::ShredTruncated);
    }
    unsafe {
        write_packed(&mut dest[..COMMON_HEADER_SIZE], header)?;
        write_packed(&mut dest[COMMON_HEADER_SIZE..CODE_SHRED_OVERHEAD], code)?;
    }
    dest[CODE_SHRED_OVERHEAD..n].copy_from_slice(payload);
    Ok(n)
}

/// Purpose: Escribe un POD packed en `dest`.
/// Inputs: `dest` — exactamente `size_of::<T>()` o más; `value` — header.
/// Returns: `Ok(())` o `ShredTruncated`.
///
/// # Safety
///
/// `T` debe ser un header packed de este módulo.
#[inline(always)]
unsafe fn write_packed<T: Copy>(dest: &mut [u8], value: T) -> Result<(), Error> {
    if dest.len() < size_of::<T>() {
        return Err(Error::ShredTruncated);
    }
    let ptr = dest.as_mut_ptr();
    if ptr.align_offset(align_of::<T>()) != 0 {
        return Err(Error::ShredTruncated);
    }
    // SAFETY: caben size_of::<T>() bytes, alineación comprobada, T es POD packed,
    // solapamos solo el prefijo de `dest`.
    unsafe {
        ptr::write_unaligned(ptr.cast::<T>(), value);
    }
    Ok(())
}

impl fmt::Debug for ShredHeader {
    /// Purpose: Debug leyendo campos por valor (evita refs packed).
    /// Inputs: `f` — formatter.
    /// Returns: `fmt::Result`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShredHeader")
            .field("slot", &self.slot())
            .field("fec_set_index", &self.fec_set_index())
            .field("index", &self.index())
            .field("version", &self.version())
            .field("shred_type", &self.shred_type())
            .field("reserved", &self.reserved())
            .finish()
    }
}

impl PartialEq for ShredHeader {
    /// Purpose: Igualdad por campos copiados.
    /// Inputs: `other` — header.
    /// Returns: `true` si todos los campos coinciden.
    fn eq(&self, other: &Self) -> bool {
        self.slot() == other.slot()
            && self.fec_set_index() == other.fec_set_index()
            && self.index() == other.index()
            && self.version() == other.version()
            && self.shred_type() == other.shred_type()
            && self.reserved() == other.reserved()
    }
}

impl Eq for ShredHeader {}

impl fmt::Debug for DataShredHeader {
    /// Purpose: Debug por valor.
    /// Inputs: `f`.
    /// Returns: `fmt::Result`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataShredHeader")
            .field("parent_offset", &self.parent_offset())
            .field("flags", &self.flags())
            .finish()
    }
}

impl PartialEq for DataShredHeader {
    /// Purpose: Igualdad por campos copiados.
    /// Inputs: `other`.
    /// Returns: `bool`.
    fn eq(&self, other: &Self) -> bool {
        self.parent_offset() == other.parent_offset() && self.flags() == other.flags()
    }
}

impl Eq for DataShredHeader {}

impl fmt::Debug for CodeShredHeader {
    /// Purpose: Debug por valor.
    /// Inputs: `f`.
    /// Returns: `fmt::Result`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodeShredHeader")
            .field("num_data", &self.num_data())
            .field("num_code", &self.num_code())
            .field("position", &self.position())
            .finish()
    }
}

impl PartialEq for CodeShredHeader {
    /// Purpose: Igualdad por campos copiados.
    /// Inputs: `other`.
    /// Returns: `bool`.
    fn eq(&self, other: &Self) -> bool {
        self.num_data() == other.num_data()
            && self.num_code() == other.num_code()
            && self.position() == other.position()
    }
}

impl Eq for CodeShredHeader {}

impl fmt::Debug for DataShred<'_> {
    /// Purpose: Debug sin volcar payloads enormes.
    /// Inputs: `f`.
    /// Returns: `fmt::Result`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataShred")
            .field("header", &self.header)
            .field("data", &self.data)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

impl fmt::Debug for CodeShred<'_> {
    /// Purpose: Debug sin volcar payloads enormes.
    /// Inputs: `f`.
    /// Returns: `fmt::Result`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodeShred")
            .field("header", &self.header)
            .field("code", &self.code)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        encode_code, encode_data, parse, CodeShredHeader, DataShredHeader, Shred, ShredHeader,
        CODE_SHRED_OVERHEAD, COMMON_HEADER_SIZE, DATA_HEADER_SIZE, DATA_SHRED_OVERHEAD,
        SHRED_TYPE_CODE, SHRED_TYPE_DATA,
    };
    use crate::arena::PacketArena;
    use crate::Error;

    /// Purpose: Tamaños packed = tamaños de cable, sin padding de rustc.
    /// Inputs: none.
    /// Returns: panics si alguien quita `packed`.
    #[test]
    fn packed_sizes_match_wire() {
        assert_eq!(COMMON_HEADER_SIZE, 20);
        assert_eq!(DATA_HEADER_SIZE, 4);
        assert_eq!(DATA_SHRED_OVERHEAD, 24);
        assert_eq!(CODE_SHRED_OVERHEAD, 26);
    }

    /// Purpose: Slice vacío no cubre el header.
    /// Inputs: none.
    /// Returns: panics si no es `ShredTruncated`.
    #[test]
    fn parse_empty_is_truncated() {
        assert_eq!(parse(&[]), Err(Error::ShredTruncated));
    }

    /// Purpose: Common header a medias.
    /// Inputs: none.
    /// Returns: panics si no es `ShredTruncated`.
    #[test]
    fn parse_short_common_header_is_truncated() {
        let buf = [0u8; COMMON_HEADER_SIZE - 1];
        assert_eq!(parse(&buf), Err(Error::ShredTruncated));
    }

    /// Purpose: Common header data sin subheader.
    /// Inputs: none.
    /// Returns: panics si no es `ShredTruncated`.
    #[test]
    fn parse_data_missing_subheader_is_truncated() {
        let mut buf = [0u8; DATA_SHRED_OVERHEAD - 1];
        buf[18] = SHRED_TYPE_DATA;
        assert_eq!(parse(&buf), Err(Error::ShredTruncated));
    }

    /// Purpose: Byte de tipo desconocido.
    /// Inputs: none.
    /// Returns: panics si no es `ShredInvalidType`.
    #[test]
    fn parse_invalid_type() {
        let mut buf = [0u8; DATA_SHRED_OVERHEAD];
        buf[18] = 0x00;
        assert_eq!(parse(&buf), Err(Error::ShredInvalidType));
    }

    /// Purpose: Encode + parse de un data shred mínimo y round-trip de campos.
    /// Inputs: none.
    /// Returns: panics si un campo o el payload no coinciden.
    #[test]
    fn data_round_trip_fields() {
        let mut buf = [0u8; 64];
        let header = ShredHeader::data(42, 7, 9, 1);
        let data = DataShredHeader::new(3, 0x80);
        let payload = b"slot-entry";
        let n = encode_data(&mut buf, header, data, payload).expect("encode");
        let shred = parse(&buf[..n]).expect("parse");
        let Shred::Data(ds) = shred else {
            panic!("expected data");
        };
        assert_eq!(ds.header(), header);
        assert_eq!(ds.data_header(), data);
        assert_eq!(ds.payload(), payload);
        assert_eq!(shred.header().slot(), 42);
        assert_eq!(shred.header().index(), 9);
        assert_eq!(shred.header().fec_set_index(), 7);
    }

    /// Purpose: El payload es el mismo pointer que el buffer (no hay copia).
    /// Inputs: none.
    /// Returns: panics si `as_ptr` difiere.
    #[test]
    fn data_payload_is_subslice() {
        let mut buf = [0u8; 64];
        let n = encode_data(
            &mut buf,
            ShredHeader::data(1, 0, 0, 0),
            DataShredHeader::new(1, 0),
            b"xyz",
        )
        .expect("encode");
        let shred = parse(&buf[..n]).expect("parse");
        assert_eq!(
            shred.payload().as_ptr(),
            buf[DATA_SHRED_OVERHEAD..n].as_ptr()
        );
    }

    /// Purpose: Encode + parse de code shred.
    /// Inputs: none.
    /// Returns: panics si FEC o payload fallan.
    #[test]
    fn code_round_trip_fields() {
        let mut buf = [0u8; 64];
        let header = ShredHeader::code(10, 4, 6, 2);
        let code = CodeShredHeader::new(2, 2, 1);
        let payload = &[0xAA, 0xBB, 0xCC];
        let n = encode_code(&mut buf, header, code, payload).expect("encode");
        let shred = parse(&buf[..n]).expect("parse");
        let Shred::Code(cs) = shred else {
            panic!("expected code");
        };
        assert_eq!(cs.header(), header);
        assert_eq!(cs.code_header(), code);
        assert_eq!(cs.payload(), payload);
        assert_eq!(cs.header().shred_type(), SHRED_TYPE_CODE);
    }

    /// Purpose: `index < fec_set_index` se rechaza.
    /// Inputs: none.
    /// Returns: panics si no es `ShredInvalidFec`.
    #[test]
    fn data_index_before_fec_set_is_invalid() {
        let mut buf = [0u8; 64];
        let err = encode_data(
            &mut buf,
            ShredHeader::data(1, 10, 3, 0),
            DataShredHeader::new(1, 0),
            &[],
        );
        assert_eq!(err, Err(Error::ShredInvalidFec));
    }

    /// Purpose: `position >= num_code` se rechaza.
    /// Inputs: none.
    /// Returns: panics si no es `ShredInvalidFec`.
    #[test]
    fn code_position_out_of_range() {
        let mut buf = [0u8; 64];
        let err = encode_code(
            &mut buf,
            ShredHeader::code(1, 0, 0, 0),
            CodeShredHeader::new(8, 2, 2),
            &[],
        );
        assert_eq!(err, Err(Error::ShredInvalidFec));
    }

    /// Purpose: `num_data == 0` es conjunto FEC vacío.
    /// Inputs: none.
    /// Returns: panics si no es `ShredInvalidFec`.
    #[test]
    fn code_zero_data_count_is_invalid() {
        let mut buf = [0u8; 64];
        let err = encode_code(
            &mut buf,
            ShredHeader::code(1, 0, 0, 0),
            CodeShredHeader::new(0, 2, 0),
            &[],
        );
        assert_eq!(err, Err(Error::ShredInvalidFec));
    }

    /// Purpose: Destino más corto que headers + payload.
    /// Inputs: none.
    /// Returns: panics si no es `ShredTruncated`.
    #[test]
    fn encode_into_too_small_dest() {
        let mut buf = [0u8; DATA_SHRED_OVERHEAD];
        let err = encode_data(
            &mut buf,
            ShredHeader::data(1, 0, 0, 0),
            DataShredHeader::new(1, 0),
            b"x",
        );
        assert_eq!(err, Err(Error::ShredTruncated));
    }

    /// Purpose: Parseo desde un slot de arena (camino real de ingestión).
    /// Inputs: none.
    /// Returns: panics si el payload no vive dentro del slot.
    #[test]
    fn parse_from_arena_slot() {
        let mut arena = PacketArena::<1>::new();
        let id = arena.acquire().expect("slot");
        let n = {
            let buf = arena.slot_mut(id).expect("mut");
            encode_data(
                buf,
                ShredHeader::data(99, 1, 1, 7),
                DataShredHeader::new(4, 0x01),
                b"hello",
            )
            .expect("encode")
        };
        arena.set_len(id, n).expect("len");
        let bytes = arena.slot(id).expect("bytes");
        let shred = parse(bytes).expect("parse");
        assert_eq!(shred.header().slot(), 99);
        assert_eq!(shred.payload(), b"hello");
        assert_eq!(
            shred.payload().as_ptr(),
            bytes[DATA_SHRED_OVERHEAD..].as_ptr()
        );
    }
}
