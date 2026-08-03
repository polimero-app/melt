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

use crate::moonraker::{FileEntry, FileEntryType, FileList, FileRoot};

use super::{MQTT_USERNAME, Profile, transport};
use transport::Error;

const PORT: u16 = 6000;
const HEADER_SIZE: usize = 16;
const MAX_PAYLOAD: usize = 16 << 20;
const MAGIC_LOGIN_CLIENT: u32 = 0x0101_013f;
const MAGIC_LOGIN_SERVER: u32 = 0x0001_013f;
const MAGIC_CTRL_CLIENT: u32 = 0x0102_013f;
const MAGIC_CTRL_SERVER: u32 = 0x0002_013f;
const MTYPE_CTRL: u32 = 12_289;
const MTYPE_SETUP: u32 = 12_291;

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
            7,
            sequence,
            json!({"peer": "studio", "peer_t": 3, "api_version": 3}),
        )?;
        let storages = reply
            .pointer("/reply/upload_storage")
            .or_else(|| reply.pointer("/reply/storage"))
            .or_else(|| reply.pointer("/reply/storage_list"))
            .and_then(Value::as_array)
            .ok_or(Error::InvalidResponse)?;
        let roots = storages
            .iter()
            .filter_map(storage_entry)
            .collect::<Vec<_>>();
        (!roots.is_empty())
            .then_some(roots)
            .ok_or(Error::InvalidResponse)
    }

    pub(super) fn list(&mut self, storage: &str, path: &str) -> Result<FileList, Error> {
        if path != "/" {
            return Err(Error::Unsupported("nested :6000 file listing"));
        }
        let sequence = self.next_sequence();
        let reply = self.request(
            1,
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
                Some(FileEntry {
                    name: name.to_owned(),
                    root,
                    device_path: format!("{root}:{path}"),
                    path,
                    entry_type: FileEntryType::File,
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
        let sequence = self.next_sequence();
        self.send_request(4, sequence, json!({"path": path, "offset": 0}))?;
        let mut copied = 0_u64;
        loop {
            let (reply, binary) = self.read_reply(4, sequence)?;
            let result = integer(reply.get("result")).ok_or(Error::InvalidResponse)?;
            let metadata = reply.pointer("/reply/mem_dl_param_size").is_some();
            if !metadata && !binary.is_empty() {
                destination.write_all(&binary).map_err(|_| Error::LocalIo)?;
                copied = copied
                    .checked_add(binary.len() as u64)
                    .ok_or(Error::FileTransfer)?;
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
        let sequence = self.next_sequence();
        self.send_request(
            2,
            sequence,
            json!({"paths": paths, "storage": storage, "api_version": 2, "peer": "studio"}),
        )?;
        let mut copied = 0_u64;
        loop {
            let (reply, binary) = self.read_reply(2, sequence)?;
            if !binary.is_empty() {
                destination.write_all(&binary).map_err(|_| Error::LocalIo)?;
                copied += binary.len() as u64;
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
        let size = source.metadata().map_err(|_| Error::LocalIo)?.len();
        let digest = file_md5(source)?;
        let sequence = self.next_sequence();
        self.send_request(
            5,
            sequence,
            json!({"type": "model", "storage": storage, "path": name, "total": size}),
        )?;
        let (init, _) = self.read_reply(5, sequence)?;
        if !matches!(integer(init.get("result")), Some(1 | 19)) {
            return Err(Error::FileTransfer);
        }
        let chunk_size = init
            .pointer("/reply/chunk_size")
            .and_then(|value| integer(Some(value)))
            .and_then(|value| usize::try_from(value).ok())
            .and_then(|value| value.checked_mul(1024))
            .filter(|value| *value > 0 && *value <= MAX_PAYLOAD)
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
                "cmdtype": 5,
                "sequence": sequence,
                "frag_id": fragment,
                "req": request,
            });
            self.send_ctrl_with_binary(&body, &buffer[..wanted])?;
            offset += wanted as u64;
            fragment = fragment.checked_add(1).ok_or(Error::FileTransfer)?;
        }
        loop {
            let (reply, _) = self.read_reply(5, sequence)?;
            match integer(reply.get("result")) {
                Some(1) => continue,
                Some(0 | 19) => return Ok(size),
                _ => return Err(Error::FileTransfer),
            }
        }
    }

    pub(super) fn delete(&mut self, storage: &str, path: &str) -> Result<(), Error> {
        let sequence = self.next_sequence();
        let reply = self.request(3, sequence, json!({"delete": [path], "storage": storage}))?;
        (integer(reply.get("result")) == Some(0))
            .then_some(())
            .ok_or(Error::FileTransfer)
    }

    fn request(&mut self, command: i64, sequence: u32, request: Value) -> Result<Value, Error> {
        self.send_request(command, sequence, request)?;
        self.read_reply(command, sequence).map(|value| value.0)
    }

    fn send_request(&mut self, command: i64, sequence: u32, request: Value) -> Result<(), Error> {
        self.send_frame(
            MAGIC_CTRL_CLIENT,
            json!({
                "mtype": MTYPE_CTRL,
                "cmdtype": command,
                "sequence": sequence,
                "req": request,
            })
            .to_string()
            .as_bytes(),
        )
    }

    fn send_ctrl_with_binary(&mut self, body: &Value, binary: &[u8]) -> Result<(), Error> {
        let json = body.to_string();
        let length = json
            .len()
            .checked_add(2)
            .and_then(|value| value.checked_add(binary.len()))
            .ok_or(Error::FileTransfer)?;
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

    fn read_reply(&mut self, command: i64, sequence: u32) -> Result<(Value, Vec<u8>), Error> {
        loop {
            let (magic, payload) = self.read_frame()?;
            if magic != MAGIC_CTRL_SERVER {
                continue;
            }
            let (json, binary) = split_payload(&payload)?;
            let reply: Value = serde_json::from_slice(json).map_err(|_| Error::InvalidResponse)?;
            if integer(reply.get("cmdtype")) == Some(command)
                && integer(reply.get("sequence")) == Some(i64::from(sequence))
            {
                return Ok((reply, binary.to_vec()));
            }
        }
    }

    fn send_frame(&mut self, magic: u32, payload: &[u8]) -> Result<(), Error> {
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
}
