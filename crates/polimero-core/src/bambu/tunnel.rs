//! BambuTunnelLocal TLS :6000 file transport used by newer printer families.

use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    time::Instant,
};

use openssl::{hash::MessageDigest, ssl::SslStream};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::moonraker::infer_file_media_type;
use crate::moonraker::{FileEntry, FileEntryType, FileList, FileRoot};

use super::{MQTT_USERNAME, Profile, transport};
use transport::Error;

const PORT: u16 = 6000;
const HEADER_SIZE: usize = 16;
const MAX_PAYLOAD: usize = 16 << 20;
const MAX_CONTROL_JSON: usize = 1 << 20;
const MAX_TRANSFER_SIZE: u64 = 64 << 30;
const MAX_SUB_FILE_SIZE: u64 = 32 << 20;
const MAX_LIST_ENTRIES: usize = 50_000;
const MAX_PATH_LENGTH: usize = 1024;
const MAX_SUB_FILE_PATHS: usize = 32;
const MAX_SKIPPED_REPLIES: usize = 64;
const MAGIC_LOGIN_CLIENT: u32 = 0x0101_013f;
const MAGIC_LOGIN_SERVER: u32 = 0x0001_013f;
const MAGIC_CTRL_CLIENT: u32 = 0x0102_013f;
const MAGIC_CTRL_SERVER: u32 = 0x0002_013f;
const MTYPE_CTRL: u32 = 12_289;
const MTYPE_SETUP: u32 = 12_291;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i64)]
enum CommandType {
    List = 1,
    SubFile = 2,
    Delete = 3,
    Download = 4,
    Upload = 5,
    Roots = 7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReplyCorrelation {
    command: CommandType,
    sequence: u32,
}

pub(super) struct Connection {
    stream: SslStream<std::net::TcpStream>,
    sequence: u32,
}

impl Connection {
    pub(super) fn open(
        profile: &Profile,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
    ) -> Result<Self, Error> {
        let access_code = access_code
            .filter(|value| !value.is_empty())
            .ok_or(Error::MissingAccessCode)?;
        if access_code.len() > 8 || access_code.chars().any(char::is_control) {
            return Err(Error::InvalidAccessCode);
        }
        let deadline = Instant::now()
            .checked_add(profile.timeout())
            .ok_or(Error::Timeout)?;
        let connector = transport::tls_connector()?;
        let (stream, _) =
            transport::open_tls(&connector, profile, PORT, fingerprint, true, deadline)?;
        let mut connection = Self {
            stream,
            sequence: 1,
        };
        let mut login = [0_u8; 16];
        login[..MQTT_USERNAME.len()].copy_from_slice(MQTT_USERNAME.as_bytes());
        login[8..8 + access_code.len()].copy_from_slice(access_code.as_bytes());
        connection.send_frame(MAGIC_LOGIN_CLIENT, &login)?;
        let (magic, _) = connection.read_frame()?;
        if magic != MAGIC_LOGIN_SERVER {
            return Err(Error::Authentication);
        }
        let setup = json!({
            "sequence": 0,
            "mtype": MTYPE_SETUP,
            "req": {
                "t_av": 1,
                "mtype": MTYPE_CTRL,
                "peer_t": 3,
                "pid": "polimero",
                "ver": env!("CARGO_PKG_VERSION"),
            }
        });
        connection.send_frame(MAGIC_CTRL_CLIENT, setup.to_string().as_bytes())?;
        let (magic, payload) = connection.read_frame()?;
        let ack: Value = serde_json::from_slice(&payload).map_err(|_| Error::InvalidResponse)?;
        if magic != MAGIC_CTRL_SERVER || integer(ack.get("result")) != Some(0) {
            return Err(Error::Authentication);
        }
        Ok(connection)
    }

    pub(super) fn roots(&mut self) -> Result<Vec<FileRoot>, Error> {
        let sequence = self.next_sequence();
        let reply = self.request(
            CommandType::Roots,
            sequence,
            json!({"peer": "studio", "peer_t": 3, "api_version": 3}),
        )?;
        let storages = reply
            .pointer("/reply/upload_storage")
            .or_else(|| reply.pointer("/reply/storage"))
            .or_else(|| reply.pointer("/reply/storage_list"))
            .and_then(Value::as_array)
            .ok_or(Error::InvalidResponse)?;
        if storages.len() > MAX_LIST_ENTRIES {
            return Err(Error::InvalidResponse);
        }
        let roots = storages
            .iter()
            .filter_map(storage_entry)
            .collect::<Vec<_>>();
        (!roots.is_empty())
            .then_some(roots)
            .ok_or(Error::InvalidResponse)
    }

    pub(super) fn list(&mut self, storage: &str, path: &str) -> Result<FileList, Error> {
        if path != "/" || !valid_wire_path(path) {
            return Err(Error::Unsupported("nested :6000 file listing"));
        }
        let sequence = self.next_sequence();
        let reply = self.request(
            CommandType::List,
            sequence,
            json!({
                "type": "model",
                "storage": storage,
                "api_version": 2,
                "notify": "DETAIL",
            }),
        )?;
        let files = reply
            .pointer("/reply/file_lists")
            .and_then(Value::as_array)
            .ok_or(Error::InvalidResponse)?;
        if files.len() > MAX_LIST_ENTRIES {
            return Err(Error::InvalidResponse);
        }
        let root = storage_root(storage)
            .map(|value| value.0)
            .ok_or(Error::InvalidDevicePath)?;
        let entries = files
            .iter()
            .filter_map(|item| {
                let wire_path = item.get("path")?.as_str()?;
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .or_else(|| wire_path.rsplit('/').next())?;
                if name.is_empty()
                    || name.contains(['/', '\\'])
                    || name.chars().any(char::is_control)
                {
                    return None;
                }
                let path = format!("/{}", wire_path.trim_start_matches('/'));
                let mut metadata = std::collections::BTreeMap::new();
                metadata.insert("mediaType".into(), Value::String("model".into()));
                for (source, target) in [
                    ("plate_count", "plateCount"),
                    ("plate_idx", "plateIndex"),
                    ("thumbnail", "thumbnail"),
                    ("thumbnail_path", "thumbnailPath"),
                ] {
                    if let Some(value) = item.get(source).filter(|value| {
                        value.is_string() || value.is_number() || value.is_boolean()
                    }) {
                        metadata.insert(target.into(), value.clone());
                    }
                }
                let media_type = infer_file_media_type(&path);
                Some(FileEntry {
                    name: name.to_owned(),
                    root,
                    device_path: format!("{root}:{path}"),
                    path,
                    entry_type: FileEntryType::File,
                    media_type,
                    size_bytes: item.get("size").and_then(|value| integer(Some(value))),
                    modified_at: item.get("time").and_then(timestamp),
                    metadata,
                })
            })
            .collect();
        Ok(FileList { entries })
    }

    pub(super) fn download(
        &mut self,
        path: &str,
        destination: &mut dyn Write,
    ) -> Result<u64, Error> {
        if !valid_wire_path(path) {
            return Err(Error::InvalidDevicePath);
        }
        let sequence = self.next_sequence();
        self.send_request(
            CommandType::Download,
            sequence,
            json!({"path": path, "offset": 0}),
        )?;
        let mut copied = 0_u64;
        loop {
            let (reply, binary) = self.read_reply(CommandType::Download, sequence)?;
            let result = integer(reply.get("result")).ok_or(Error::InvalidResponse)?;
            let metadata = reply.pointer("/reply/mem_dl_param_size").is_some();
            if !metadata && !binary.is_empty() {
                destination.write_all(&binary).map_err(|_| Error::LocalIo)?;
                copied = copied
                    .checked_add(binary.len() as u64)
                    .ok_or(Error::FileTransfer)?;
                if copied > MAX_TRANSFER_SIZE {
                    return Err(Error::FileTransfer);
                }
            }
            match result {
                1 => continue,
                0 => return Ok(copied),
                _ => return Err(Error::FileTransfer),
            }
        }
    }

    pub(super) fn sub_file(
        &mut self,
        storage: &str,
        paths: &[String],
        destination: &mut dyn Write,
    ) -> Result<u64, Error> {
        if paths.is_empty()
            || paths.len() > MAX_SUB_FILE_PATHS
            || paths.iter().any(|path| !valid_wire_path(path))
        {
            return Err(Error::InvalidDevicePath);
        }
        let sequence = self.next_sequence();
        self.send_request(
            CommandType::SubFile,
            sequence,
            json!({"paths": paths, "storage": storage, "api_version": 2, "peer": "studio"}),
        )?;
        let mut copied = 0_u64;
        loop {
            let (reply, binary) = self.read_reply(CommandType::SubFile, sequence)?;
            if !binary.is_empty() {
                destination.write_all(&binary).map_err(|_| Error::LocalIo)?;
                copied = copied
                    .checked_add(binary.len() as u64)
                    .ok_or(Error::FileTransfer)?;
                if copied > MAX_SUB_FILE_SIZE {
                    return Err(Error::FileTransfer);
                }
            }
            match integer(reply.get("result")) {
                Some(1) => continue,
                Some(0) if copied > 0 => return Ok(copied),
                _ => return Err(Error::FileTransfer),
            }
        }
    }

    pub(super) fn upload(
        &mut self,
        storage: &str,
        source: &Path,
        name: &str,
    ) -> Result<u64, Error> {
        if !valid_wire_path(name) {
            return Err(Error::InvalidDevicePath);
        }
        let size = source.metadata().map_err(|_| Error::LocalIo)?.len();
        if size > MAX_TRANSFER_SIZE {
            return Err(Error::FileTransfer);
        }
        let digest = file_md5(source)?;
        let sequence = self.next_sequence();
        self.send_request(
            CommandType::Upload,
            sequence,
            json!({"type": "model", "storage": storage, "path": name, "total": size}),
        )?;
        let (init, _) = self.read_reply(CommandType::Upload, sequence)?;
        if !matches!(integer(init.get("result")), Some(1 | 19)) {
            return Err(Error::FileTransfer);
        }
        let chunk_size = init
            .pointer("/reply/chunk_size")
            .and_then(|value| integer(Some(value)))
            .and_then(|value| usize::try_from(value).ok())
            .and_then(|value| value.checked_mul(1024))
            .filter(|value| *value > 0 && *value <= MAX_PAYLOAD - 4096)
            .ok_or(Error::InvalidResponse)?;
        let mut source = File::open(source).map_err(|_| Error::LocalIo)?;
        let mut offset = init
            .pointer("/reply/offset")
            .and_then(|value| integer(Some(value)))
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(0);
        if offset > size {
            return Err(Error::InvalidResponse);
        }
        if offset > 0 {
            use std::io::Seek;
            source
                .seek(std::io::SeekFrom::Start(offset))
                .map_err(|_| Error::LocalIo)?;
        }
        let mut fragment = 0_u32;
        let mut buffer = vec![0; chunk_size];
        while offset < size {
            let wanted = usize::try_from((size - offset).min(chunk_size as u64))
                .map_err(|_| Error::FileTransfer)?;
            source
                .read_exact(&mut buffer[..wanted])
                .map_err(|_| Error::LocalIo)?;
            let last = offset + wanted as u64 == size;
            let mut request = json!({"offset": offset, "size": wanted});
            if last {
                request["file_md5"] = Value::String(digest.clone());
            }
            let body = json!({
                "mtype": MTYPE_CTRL,
                "cmdtype": CommandType::Upload as i64,
                "sequence": sequence,
                "frag_id": fragment,
                "req": request,
            });
            self.send_ctrl_with_binary(&body, &buffer[..wanted])?;
            offset += wanted as u64;
            fragment = fragment.checked_add(1).ok_or(Error::FileTransfer)?;
        }
        loop {
            let (reply, _) = self.read_reply(CommandType::Upload, sequence)?;
            match integer(reply.get("result")) {
                Some(1) => continue,
                Some(0 | 19) => return Ok(size),
                _ => return Err(Error::FileTransfer),
            }
        }
    }

    pub(super) fn delete(&mut self, storage: &str, path: &str) -> Result<(), Error> {
        if !valid_wire_path(path) {
            return Err(Error::InvalidDevicePath);
        }
        let sequence = self.next_sequence();
        let reply = self.request(
            CommandType::Delete,
            sequence,
            json!({"delete": [path], "storage": storage}),
        )?;
        (integer(reply.get("result")) == Some(0))
            .then_some(())
            .ok_or(Error::FileTransfer)
    }

    fn request(
        &mut self,
        command: CommandType,
        sequence: u32,
        request: Value,
    ) -> Result<Value, Error> {
        self.send_request(command, sequence, request)?;
        self.read_reply(command, sequence).map(|value| value.0)
    }

    fn send_request(
        &mut self,
        command: CommandType,
        sequence: u32,
        request: Value,
    ) -> Result<(), Error> {
        self.send_frame(
            MAGIC_CTRL_CLIENT,
            json!({
                "mtype": MTYPE_CTRL,
                "cmdtype": command as i64,
                "sequence": sequence,
                "req": request,
            })
            .to_string()
            .as_bytes(),
        )
    }

    fn send_ctrl_with_binary(&mut self, body: &Value, binary: &[u8]) -> Result<(), Error> {
        let json = body.to_string();
        if json.len() > MAX_CONTROL_JSON || binary.len() > MAX_PAYLOAD {
            return Err(Error::FileTransfer);
        }
        let length = json
            .len()
            .checked_add(2)
            .and_then(|value| value.checked_add(binary.len()))
            .ok_or(Error::FileTransfer)?;
        if length > MAX_PAYLOAD {
            return Err(Error::FileTransfer);
        }
        self.send_header(MAGIC_CTRL_CLIENT, length)?;
        self.stream
            .write_all(json.as_bytes())
            .map_err(|_| Error::Connection)?;
        self.stream
            .write_all(b"\n\n")
            .map_err(|_| Error::Connection)?;
        self.stream
            .write_all(binary)
            .map_err(|_| Error::Connection)?;
        self.stream.flush().map_err(|_| Error::Connection)
    }

    fn read_reply(
        &mut self,
        command: CommandType,
        sequence: u32,
    ) -> Result<(Value, Vec<u8>), Error> {
        let correlation = ReplyCorrelation { command, sequence };
        for _ in 0..MAX_SKIPPED_REPLIES {
            let (magic, payload) = self.read_frame()?;
            if magic != MAGIC_CTRL_SERVER {
                continue;
            }
            let (json, binary) = split_payload(&payload)?;
            if json.len() > MAX_CONTROL_JSON {
                return Err(Error::InvalidResponse);
            }
            let reply: Value = serde_json::from_slice(json).map_err(|_| Error::InvalidResponse)?;
            if reply_matches(&reply, correlation) {
                return Ok((reply, binary.to_vec()));
            }
        }
        Err(Error::InvalidResponse)
    }

    fn send_frame(&mut self, magic: u32, payload: &[u8]) -> Result<(), Error> {
        if payload.len() > MAX_PAYLOAD {
            return Err(Error::FileTransfer);
        }
        self.send_header(magic, payload.len())?;
        self.stream
            .write_all(payload)
            .map_err(|_| Error::Connection)?;
        self.stream.flush().map_err(|_| Error::Connection)
    }

    fn send_header(&mut self, magic: u32, length: usize) -> Result<(), Error> {
        let length = u32::try_from(length).map_err(|_| Error::FileTransfer)?;
        let mut header = [0_u8; HEADER_SIZE];
        header[..4].copy_from_slice(&length.to_le_bytes());
        header[4..8].copy_from_slice(&magic.to_le_bytes());
        header[8..12].copy_from_slice(&self.sequence.to_le_bytes());
        self.sequence = self.sequence.wrapping_add(1);
        self.stream
            .write_all(&header)
            .map_err(|_| Error::Connection)
    }

    fn read_frame(&mut self) -> Result<(u32, Vec<u8>), Error> {
        let mut header = [0_u8; HEADER_SIZE];
        self.stream
            .read_exact(&mut header)
            .map_err(|_| Error::Connection)?;
        let length = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        if length > MAX_PAYLOAD {
            return Err(Error::InvalidResponse);
        }
        let magic = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let mut payload = vec![0; length];
        self.stream
            .read_exact(&mut payload)
            .map_err(|_| Error::Connection)?;
        Ok((magic, payload))
    }

    fn next_sequence(&mut self) -> u32 {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        sequence
    }
}

fn storage_entry(value: &Value) -> Option<FileRoot> {
    let wire_name = value
        .as_str()
        .or_else(|| value.get("name").and_then(Value::as_str))
        .or_else(|| value.get("storage").and_then(Value::as_str))?;
    let (name, description) = storage_root(wire_name)?;
    let total = value
        .get("total")
        .or_else(|| value.get("capacity"))
        .and_then(|value| integer(Some(value)))
        .and_then(|value| value.try_into().ok());
    let free = value
        .get("free")
        .or_else(|| value.get("available"))
        .and_then(|value| integer(Some(value)))
        .and_then(|value| value.try_into().ok());
    Some(FileRoot {
        name,
        description,
        writable: value
            .get("writable")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        capacity_bytes: total,
        free_bytes: free,
        metadata: Default::default(),
    })
}

fn timestamp(value: &Value) -> Option<String> {
    let seconds = integer(Some(value))?;
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

fn split_payload(payload: &[u8]) -> Result<(&[u8], &[u8]), Error> {
    let mut depth = 0_i32;
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in payload.iter().copied().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let end = index + 1;
                    let binary = payload
                        .get(end..)
                        .and_then(|tail| tail.strip_prefix(b"\n\n"))
                        .unwrap_or(&payload[end..]);
                    return Ok((&payload[..end], binary));
                }
            }
            _ => {}
        }
    }
    Err(Error::InvalidResponse)
}

fn storage_root(value: &str) -> Option<(&'static str, &'static str)> {
    match value.to_ascii_lowercase().as_str() {
        "emmc" | "internal" => Some(("emmc", "Internal storage")),
        "udisk" | "usb" | "external" => Some(("udisk", "USB storage")),
        "sdcard" => Some(("sdcard", "SD card")),
        _ => None,
    }
}

fn integer(value: Option<&Value>) -> Option<i64> {
    value?.as_i64().or_else(|| {
        value?
            .as_u64()
            .and_then(|number| i64::try_from(number).ok())
    })
}

fn reply_matches(reply: &Value, correlation: ReplyCorrelation) -> bool {
    integer(reply.get("cmdtype")) == Some(correlation.command as i64)
        && integer(reply.get("sequence")) == Some(i64::from(correlation.sequence))
}

fn valid_wire_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH_LENGTH
        && !path.chars().any(char::is_control)
        && !path.contains('\\')
        && !path.split(['/', '#']).any(|component| component == "..")
}

fn file_md5(path: &Path) -> Result<String, Error> {
    let mut file = File::open(path).map_err(|_| Error::LocalIo)?;
    let mut hasher =
        openssl::hash::Hasher::new(MessageDigest::md5()).map_err(|_| Error::LocalIo)?;
    let mut buffer = [0_u8; 64 << 10];
    loop {
        let read = file.read(&mut buffer).map_err(|_| Error::LocalIo)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]).map_err(|_| Error::LocalIo)?;
    }
    let digest = hasher.finish().map_err(|_| Error::LocalIo)?;
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_json_from_binary_without_stripping_file_bytes() {
        let payload = b"{\"reply\":{\"text\":\"}\\\"\"}}\n\n\nfile";
        let (json, binary) = split_payload(payload).unwrap();
        assert_eq!(json, b"{\"reply\":{\"text\":\"}\\\"\"}}");
        assert_eq!(binary, b"\nfile");
    }

    #[test]
    fn normalizes_storage_aliases() {
        assert_eq!(storage_root("internal").unwrap().0, "emmc");
        assert_eq!(storage_root("usb").unwrap().0, "udisk");
        assert!(storage_root("future").is_none());
    }

    #[test]
    fn preserves_storage_capacity_and_file_timestamps() {
        let root = storage_entry(&json!({
            "name": "internal",
            "capacity": 1_000_000,
            "available": 250_000,
            "writable": false
        }))
        .unwrap();
        assert_eq!(root.name, "emmc");
        assert_eq!(root.capacity_bytes, Some(1_000_000));
        assert_eq!(root.free_bytes, Some(250_000));
        assert!(!root.writable);
        assert_eq!(
            timestamp(&json!(0)).as_deref(),
            Some("1970-01-01T00:00:00Z")
        );
    }

    #[test]
    fn reply_correlation_requires_command_and_sequence() {
        let expected = ReplyCorrelation {
            command: CommandType::Upload,
            sequence: 7,
        };
        assert!(reply_matches(&json!({"cmdtype":5,"sequence":7}), expected));
        assert!(!reply_matches(&json!({"cmdtype":4,"sequence":7}), expected));
        assert!(!reply_matches(&json!({"cmdtype":5,"sequence":8}), expected));
    }

    #[test]
    fn wire_paths_are_bounded_and_cannot_escape() {
        assert!(valid_wire_path("/cache/part.3mf#thumbnail"));
        assert!(!valid_wire_path("../secret"));
        assert!(!valid_wire_path("/cache\\part.3mf"));
        assert!(!valid_wire_path(&"x".repeat(MAX_PATH_LENGTH + 1)));
    }

    #[test]
    fn executable_upload_transcript_keeps_frag_id_top_level() {
        let transcript: Value = serde_json::from_str(include_str!(
            "../../../../fixtures/bambu/tunnel-6000/upload-frag-id/transcript.json"
        ))
        .unwrap();
        let client = &transcript["client"];
        assert_eq!(client["frag_id"], 0);
        assert!(client["req"].get("frag_id").is_none());
        assert!(reply_matches(
            &transcript["server"],
            ReplyCorrelation {
                command: CommandType::Upload,
                sequence: 7,
            }
        ));
    }
}
