use std::io::Read;

pub const REDIRECT_MIME_TYPE: u16 = 0xffff;
pub const LINK_TARGET_MIME_TYPE: u16 = 0xfffe;
pub const DELETED_MIME_TYPE: u16 = 0xfffd;

#[derive(Debug)]
pub enum DirentData {
    Content {
        cluster_number: u32,
        blob_number: u32,
    },
    Redirect {
        redirect_index: u32,
    },
    LinkTarget,
    Deleted,
}

#[derive(Debug)]
pub struct Dirent {
    pub mime_type: u16,
    pub extra_len: u8,
    pub namespace: char,
    pub revision: u32,
    pub data: DirentData,
    pub url: String,
    pub title: String,
    pub parameter: Vec<u8>,
}

impl Dirent {
    pub fn parse(mut reader: impl Read) -> Result<Self, String> {
        let mut fixed_buf = [0u8; 8];
        reader.read_exact(&mut fixed_buf).map_err(|e| e.to_string())?;

        let mime_type = u16::from_le_bytes(fixed_buf[0..2].try_into().unwrap());
        let extra_len = fixed_buf[2];
        let namespace = fixed_buf[3] as char;
        let revision = u32::from_le_bytes(fixed_buf[4..8].try_into().unwrap());

        let data = match mime_type {
            REDIRECT_MIME_TYPE => {
                let mut buf = [0u8; 4];
                reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
                DirentData::Redirect {
                    redirect_index: u32::from_le_bytes(buf),
                }
            }
            LINK_TARGET_MIME_TYPE => DirentData::LinkTarget,
            DELETED_MIME_TYPE => DirentData::Deleted,
            _ => {
                let mut buf = [0u8; 8];
                reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
                DirentData::Content {
                    cluster_number: u32::from_le_bytes(buf[0..4].try_into().unwrap()),
                    blob_number: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
                }
            }
        };

        let url = read_null_terminated_string(&mut reader)?;
        let title = read_null_terminated_string(&mut reader)?;

        let mut parameter = vec![0u8; extra_len as usize];
        if extra_len > 0 {
            reader.read_exact(&mut parameter).map_err(|e| e.to_string())?;
        }

        Ok(Dirent {
            mime_type,
            extra_len,
            namespace,
            revision,
            data,
            url,
            title,
            parameter,
        })
    }

    pub fn is_redirect(&self) -> bool {
        self.mime_type == REDIRECT_MIME_TYPE
    }

    pub fn is_link_target(&self) -> bool {
        self.mime_type == LINK_TARGET_MIME_TYPE
    }

    pub fn is_deleted(&self) -> bool {
        self.mime_type == DELETED_MIME_TYPE
    }

    pub fn is_article(&self) -> bool {
        !self.is_redirect() && !self.is_link_target() && !self.is_deleted()
    }

    pub fn get_title(&self) -> &str {
        if self.title.is_empty() {
            &self.url
        } else {
            &self.title
        }
    }
}

fn read_null_terminated_string(reader: &mut impl Read) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        match reader.read_exact(&mut buf) {
            Ok(_) => {
                if buf[0] == 0 {
                    break;
                }
                bytes.push(buf[0]);
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_read_null_terminated_string() {
        let data = b"hello\0world\0";
        let mut reader = Cursor::new(data);
        assert_eq!(read_null_terminated_string(&mut reader).unwrap(), "hello");
        assert_eq!(read_null_terminated_string(&mut reader).unwrap(), "world");
    }

    #[test]
    fn test_parse_content_entry_dirent() {
        let mut data = Vec::new();
        // mime_type: 1 (article)
        data.extend_from_slice(&1u16.to_le_bytes());
        // extra_len: 0
        data.push(0);
        // namespace: 'C'
        data.push(b'C');
        // revision: 123
        data.extend_from_slice(&123u32.to_le_bytes());
        // cluster_number: 10, blob_number: 20
        data.extend_from_slice(&10u32.to_le_bytes());
        data.extend_from_slice(&20u32.to_le_bytes());
        // url: "foo\0"
        data.extend_from_slice(b"foo\0");
        // title: "Bar\0"
        data.extend_from_slice(b"Bar\0");

        let mut reader = Cursor::new(data);
        let dirent = Dirent::parse(&mut reader).unwrap();

        assert_eq!(dirent.mime_type, 1);
        assert_eq!(dirent.extra_len, 0);
        assert_eq!(dirent.namespace, 'C');
        assert_eq!(dirent.revision, 123);
        assert!(dirent.is_article());
        if let DirentData::Content { cluster_number, blob_number } = dirent.data {
            assert_eq!(cluster_number, 10);
            assert_eq!(blob_number, 20);
        } else {
            panic!("Expected Content data");
        }
        assert_eq!(dirent.url, "foo");
        assert_eq!(dirent.title, "Bar");
        assert_eq!(dirent.get_title(), "Bar");
    }

    #[test]
    fn test_parse_redirect_dirent() {
        let mut data = Vec::new();
        // mime_type: 0xffff (redirect)
        data.extend_from_slice(&REDIRECT_MIME_TYPE.to_le_bytes());
        // extra_len: 0
        data.push(0);
        // namespace: 'R'
        data.push(b'R');
        // revision: 0
        data.extend_from_slice(&0u32.to_le_bytes());
        // redirect_index: 500
        data.extend_from_slice(&500u32.to_le_bytes());
        // url: "redir\0"
        data.extend_from_slice(b"redir\0");
        // title: "\0" (empty title)
        data.extend_from_slice(b"\0");

        let mut reader = Cursor::new(data);
        let dirent = Dirent::parse(&mut reader).unwrap();

        assert!(dirent.is_redirect());
        if let DirentData::Redirect { redirect_index } = dirent.data {
            assert_eq!(redirect_index, 500);
        } else {
            panic!("Expected Redirect data");
        }
        assert_eq!(dirent.url, "redir");
        assert_eq!(dirent.title, "");
        assert_eq!(dirent.get_title(), "redir");
    }

    #[test]
    fn test_parse_dirent_with_parameter() {
        let mut data = Vec::new();
        // mime_type: 1, extra_len: 4
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(4);
        data.push(b'C');
        data.extend_from_slice(&0u32.to_le_bytes());
        // cluster_number: 0, blob_number: 0
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        // url: "a\0", title: "b\0"
        data.extend_from_slice(b"a\0");
        data.extend_from_slice(b"b\0");
        // parameter: [1, 2, 3, 4]
        data.extend_from_slice(&[1, 2, 3, 4]);

        let mut reader = Cursor::new(data);
        let dirent = Dirent::parse(&mut reader).unwrap();

        assert_eq!(dirent.extra_len, 4);
        assert_eq!(dirent.parameter, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_parse_link_target_dirent() {
        let mut data = Vec::new();
        data.extend_from_slice(&LINK_TARGET_MIME_TYPE.to_le_bytes());
        data.push(0); // extra_len
        data.push(b'L'); // namespace
        data.extend_from_slice(&0u32.to_le_bytes()); // revision
        data.extend_from_slice(b"link\0"); // url
        data.extend_from_slice(b"Title\0"); // title

        let mut reader = Cursor::new(data);
        let dirent = Dirent::parse(&mut reader).unwrap();

        assert!(dirent.is_link_target());
        assert!(matches!(dirent.data, DirentData::LinkTarget));
    }

    #[test]
    fn test_parse_deleted_dirent() {
        let mut data = Vec::new();
        data.extend_from_slice(&DELETED_MIME_TYPE.to_le_bytes());
        data.push(0); // extra_len
        data.push(b'D'); // namespace
        data.extend_from_slice(&0u32.to_le_bytes()); // revision
        data.extend_from_slice(b"gone\0"); // url
        data.extend_from_slice(b"Gone\0"); // title

        let mut reader = Cursor::new(data);
        let dirent = Dirent::parse(&mut reader).unwrap();

        assert!(dirent.is_deleted());
        assert!(matches!(dirent.data, DirentData::Deleted));
    }
}
