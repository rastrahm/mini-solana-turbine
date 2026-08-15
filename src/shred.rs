//! Layout binario de shreds (data vs code) y parseo zero-copy.
//!
//! El payload es un subslice del buffer de entrada (slot de arena). Los headers
//! se leen como `#[repr(C, packed)]` ([`ShredHeader`]): align 1, acceso a campos solo
//! por valor (nunca `&header.slot`).
//!
//! La firma Ed25519 es educativa: `64 B` de firma **delante** del body (mismo
//! esquema que Solana: se firma el resto del paquete). No incluye merkle root,
//! `ShredVariant` ni el wire format de mainnet.

use crate::Error;
use core::fmt;
use core::mem::{align_of, size_of};
use core::ptr;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

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
/// Bytes de una firma Ed25519 (prefijo educativo del paquete firmado).
pub const SIGNATURE_BYTES: usize = 64;
/// Bytes de una clave pública Ed25519.
pub const PUBLIC_KEY_BYTES: usize = 32;
/// Bytes de una semilla / clave secreta Ed25519.
pub const SECRET_KEY_BYTES: usize = 32;

/// Clave pública del líder (32 B). No es el pubkey de mainnet Solana.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ShredPublicKey([u8; PUBLIC_KEY_BYTES]);

/// Semilla Ed25519 del firmante. Debug no vuelca los bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ShredSecretKey([u8; SECRET_KEY_BYTES]);

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

impl ShredPublicKey {
    /// Purpose: Construye una pubkey desde 32 bytes crudos.
    /// Inputs: `bytes` — punto comprimido Ed25519.
    /// Returns: la clave; la validez geométrica se comprueba al verificar.
    #[inline(always)]
    pub const fn from_bytes(bytes: [u8; PUBLIC_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Purpose: Bytes del punto.
    /// Inputs: none (`self` por valor).
    /// Returns: `[u8; 32]`.
    #[inline(always)]
    pub const fn to_bytes(self) -> [u8; PUBLIC_KEY_BYTES] {
        self.0
    }
}

impl ShredSecretKey {
    /// Purpose: Semilla de 32 bytes (cualquier valor es una seed válida).
    /// Inputs: `bytes` — entropy / test vector.
    /// Returns: clave para [`encode_signed_data`] / [`encode_signed_code`].
    #[inline(always)]
    pub const fn from_bytes(bytes: [u8; SECRET_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Purpose: Pubkey derivada (clamping de Ed25519).
    /// Inputs: none (`&self`).
    /// Returns: [`ShredPublicKey`].
    pub fn public(&self) -> ShredPublicKey {
        let sk = SigningKey::from_bytes(&self.0);
        ShredPublicKey(sk.verifying_key().to_bytes())
    }
}

impl fmt::Debug for ShredPublicKey {
    /// Purpose: Debug hex corto.
    /// Inputs: `f`.
    /// Returns: `fmt::Result`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ShredPublicKey").field(&self.0).finish()
    }
}

impl fmt::Debug for ShredSecretKey {
    /// Purpose: No filtra la semilla.
    /// Inputs: `f`.
    /// Returns: `fmt::Result`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShredSecretKey(..)")
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

/// Purpose: Firma el body y lo escribe como `sig[64] || body` (educativo).
/// Inputs: `dest` — slot; `secret` — semilla; `body` — paquete ya encodeado
///   (salida de [`encode_data`] / [`encode_code`]).
/// Returns: `64 + body.len()`, o `ShredTruncated`.
pub fn attach_signature(
    dest: &mut [u8],
    secret: &ShredSecretKey,
    body: &[u8],
) -> Result<usize, Error> {
    let n = SIGNATURE_BYTES + body.len();
    if dest.len() < n {
        return Err(Error::ShredTruncated);
    }
    dest[SIGNATURE_BYTES..n].copy_from_slice(body);
    let (sig_buf, rest) = dest.split_at_mut(SIGNATURE_BYTES);
    write_signature(sig_buf, secret, &rest[..body.len()])?;
    Ok(n)
}

/// Purpose: Encode data + firma en el mismo slot (prefijo 64 B).
/// Inputs: igual que [`encode_data`] más `secret`.
/// Returns: longitud total (`64 + overhead + payload`).
pub fn encode_signed_data(
    dest: &mut [u8],
    secret: &ShredSecretKey,
    header: ShredHeader,
    data: DataShredHeader,
    payload: &[u8],
) -> Result<usize, Error> {
    if dest.len() < SIGNATURE_BYTES {
        return Err(Error::ShredTruncated);
    }
    let n = encode_data(&mut dest[SIGNATURE_BYTES..], header, data, payload)?;
    let (sig_buf, rest) = dest.split_at_mut(SIGNATURE_BYTES);
    write_signature(sig_buf, secret, &rest[..n])?;
    Ok(SIGNATURE_BYTES + n)
}

/// Purpose: Encode code + firma en el mismo slot.
/// Inputs: igual que [`encode_code`] más `secret`.
/// Returns: longitud total.
pub fn encode_signed_code(
    dest: &mut [u8],
    secret: &ShredSecretKey,
    header: ShredHeader,
    code: CodeShredHeader,
    payload: &[u8],
) -> Result<usize, Error> {
    if dest.len() < SIGNATURE_BYTES {
        return Err(Error::ShredTruncated);
    }
    let n = encode_code(&mut dest[SIGNATURE_BYTES..], header, code, payload)?;
    let (sig_buf, rest) = dest.split_at_mut(SIGNATURE_BYTES);
    write_signature(sig_buf, secret, &rest[..n])?;
    Ok(SIGNATURE_BYTES + n)
}

/// Purpose: Verifica Ed25519 y devuelve el body (zero-copy, sin firma).
/// Inputs: `bytes` — `sig || body`; `public` — líder.
/// Returns: subslice `bytes[64..]` si la firma es válida; `ShredTruncated` /
///   `ShredInvalidKey` / `ShredBadSignature`.
pub fn verify_signed<'a>(bytes: &'a [u8], public: &ShredPublicKey) -> Result<&'a [u8], Error> {
    if bytes.len() < SIGNATURE_BYTES {
        return Err(Error::ShredTruncated);
    }
    let sig_bytes: [u8; SIGNATURE_BYTES] = match bytes[..SIGNATURE_BYTES].try_into() {
        Ok(arr) => arr,
        Err(_) => return Err(Error::ShredTruncated),
    };
    let vk = VerifyingKey::from_bytes(&public.0).map_err(|_| Error::ShredInvalidKey)?;
    let sig = Signature::from_bytes(&sig_bytes);
    let body = &bytes[SIGNATURE_BYTES..];
    vk.verify_strict(body, &sig)
        .map_err(|_| Error::ShredBadSignature)?;
    Ok(body)
}

/// Purpose: Verifica la firma y parsea el body (payload sigue prestado del slot).
/// Inputs: `bytes` — paquete firmado; `public` — líder.
/// Returns: [`Shred`] o error de firma / parseo.
pub fn parse_signed<'a>(bytes: &'a [u8], public: &ShredPublicKey) -> Result<Shred<'a>, Error> {
    parse(verify_signed(bytes, public)?)
}

/// Purpose: Escribe 64 B de firma Ed25519 de `body` en `sig_out`.
/// Inputs: `sig_out` — ≥ 64 B; `secret`; `body` — mensaje (el shred sin firma).
/// Returns: `Ok` o `ShredTruncated`.
fn write_signature(sig_out: &mut [u8], secret: &ShredSecretKey, body: &[u8]) -> Result<(), Error> {
    if sig_out.len() < SIGNATURE_BYTES {
        return Err(Error::ShredTruncated);
    }
    let sk = SigningKey::from_bytes(&secret.0);
    let sig = sk.sign(body);
    sig_out[..SIGNATURE_BYTES].copy_from_slice(&sig.to_bytes());
    Ok(())
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
        attach_signature, encode_code, encode_data, encode_signed_code, encode_signed_data, parse,
        parse_signed, CodeShredHeader, DataShredHeader, Shred, ShredHeader, ShredSecretKey,
        CODE_SHRED_OVERHEAD, COMMON_HEADER_SIZE, DATA_HEADER_SIZE, DATA_SHRED_OVERHEAD,
        SECRET_KEY_BYTES, SHRED_TYPE_CODE, SHRED_TYPE_DATA, SIGNATURE_BYTES,
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

    /// Purpose: Seed fija produce una pubkey estable.
    /// Inputs: none.
    /// Returns: panics si Debug de la secreta filtra bytes.
    #[test]
    fn secret_debug_hides_seed() {
        let sk = ShredSecretKey::from_bytes([3u8; SECRET_KEY_BYTES]);
        assert_eq!(format!("{:?}", sk), "ShredSecretKey(..)");
    }

    /// Purpose: Firma educativa + parse_signed round-trip, payload zero-copy.
    /// Inputs: none.
    /// Returns: panics si no verifica o el payload no apunta al body.
    #[test]
    fn signed_data_round_trip() {
        let sk = ShredSecretKey::from_bytes([7u8; SECRET_KEY_BYTES]);
        let pk = sk.public();
        let mut buf = [0u8; 128];
        let n = encode_signed_data(
            &mut buf,
            &sk,
            ShredHeader::data(1, 0, 0, 1),
            DataShredHeader::new(1, 0),
            b"sig-body",
        )
        .expect("encode signed");
        let shred = parse_signed(&buf[..n], &pk).expect("verify parse");
        let Shred::Data(ds) = shred else {
            panic!("expected data");
        };
        assert_eq!(ds.payload(), b"sig-body");
        assert_eq!(
            shred.payload().as_ptr(),
            buf[SIGNATURE_BYTES + DATA_SHRED_OVERHEAD..n].as_ptr()
        );
        assert_eq!(parse(&buf[..n]).err(), Some(Error::ShredInvalidType));
    }

    /// Purpose: `attach_signature` sobre un body ya encodeado equivale a encode_signed.
    /// Inputs: none.
    /// Returns: panics si las longitudes o el parse difieren.
    #[test]
    fn attach_signature_over_encoded_body() {
        let sk = ShredSecretKey::from_bytes([8u8; SECRET_KEY_BYTES]);
        let mut body = [0u8; 64];
        let nb = encode_data(
            &mut body,
            ShredHeader::data(1, 0, 0, 1),
            DataShredHeader::new(1, 0),
            b"att",
        )
        .expect("body");
        let mut pkt = [0u8; 128];
        let n = attach_signature(&mut pkt, &sk, &body[..nb]).expect("attach");
        parse_signed(&pkt[..n], &sk.public()).expect("verify");
        assert_eq!(n, SIGNATURE_BYTES + nb);
    }

    /// Purpose: Bit flip en el body invalida la firma.
    /// Inputs: none.
    /// Returns: panics si no es `ShredBadSignature`.
    #[test]
    fn signed_rejects_tampered_body() {
        let sk = ShredSecretKey::from_bytes([9u8; SECRET_KEY_BYTES]);
        let pk = sk.public();
        let mut buf = [0u8; 128];
        let n = encode_signed_data(
            &mut buf,
            &sk,
            ShredHeader::data(1, 0, 0, 1),
            DataShredHeader::new(1, 0),
            b"aaaa",
        )
        .expect("encode");
        buf[n - 1] ^= 0x01;
        assert_eq!(parse_signed(&buf[..n], &pk), Err(Error::ShredBadSignature));
    }

    /// Purpose: Otra seed no verifica.
    /// Inputs: none.
    /// Returns: panics si no es `ShredBadSignature`.
    #[test]
    fn signed_rejects_wrong_key() {
        let sk = ShredSecretKey::from_bytes([1u8; SECRET_KEY_BYTES]);
        let other = ShredSecretKey::from_bytes([2u8; SECRET_KEY_BYTES]).public();
        let mut buf = [0u8; 128];
        let n = encode_signed_data(
            &mut buf,
            &sk,
            ShredHeader::data(1, 0, 0, 1),
            DataShredHeader::new(1, 0),
            b"x",
        )
        .expect("encode");
        assert_eq!(
            parse_signed(&buf[..n], &other),
            Err(Error::ShredBadSignature)
        );
    }

    /// Purpose: Menos de 64 B no cubre la firma.
    /// Inputs: none.
    /// Returns: panics si no es `ShredTruncated`.
    #[test]
    fn signed_truncated_is_truncated() {
        let pk = ShredSecretKey::from_bytes([1u8; SECRET_KEY_BYTES]).public();
        assert_eq!(
            parse_signed(&[0u8; SIGNATURE_BYTES - 1], &pk),
            Err(Error::ShredTruncated)
        );
    }

    /// Purpose: Code shred firmado parsea.
    /// Inputs: none.
    /// Returns: panics si el payload no coincide.
    #[test]
    fn signed_code_round_trip() {
        let sk = ShredSecretKey::from_bytes([4u8; SECRET_KEY_BYTES]);
        let mut buf = [0u8; 128];
        let n = encode_signed_code(
            &mut buf,
            &sk,
            ShredHeader::code(1, 0, 1, 1),
            CodeShredHeader::new(2, 1, 0),
            &[0x11, 0x22],
        )
        .expect("encode code");
        let shred = parse_signed(&buf[..n], &sk.public()).expect("parse");
        let Shred::Code(cs) = shred else {
            panic!("expected code");
        };
        assert_eq!(cs.payload(), &[0x11, 0x22]);
    }
}
