// Adding a sound from a link: a sound page (Voicemod Tuna, MyInstants, any page describing its
// sound in schema.org or Open Graph data) or a direct link to an audio file. Fetched over HTTPS
// only, with Windows' own HTTP stack (system proxy and certificates), size-capped, and kept
// only if it decodes as audio.
use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;
use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_CONTENT_LENGTH,
    WINHTTP_QUERY_CONTENT_TYPE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::{PCWSTR, w};

use crate::soundpad::decode_entities;
use crate::store::AUDIO_EXT;

const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SOUND_BYTES: usize = 50 * 1024 * 1024;
const TIMEOUT_MS: i32 = 15_000;

pub struct Downloaded {
    pub path: PathBuf,
    pub title: String,
    pub duration: f64,
}

/// Downloads the sound behind `link` into `dir`. `known(title)` refuses a sound already there.
pub fn download(
    link: &str,
    dir: &Path,
    known: impl Fn(&str) -> bool,
) -> Result<Downloaded, String> {
    let link = link.trim();
    let first = get(link, MAX_SOUND_BYTES)?;
    let (title, audio_url, audio) = if is_audio(&first.content_type, link) {
        (title_from_url(link), link.to_string(), first)
    } else {
        if first.body.len() > MAX_PAGE_BYTES {
            return Err("page trop grosse".into());
        }
        let page = String::from_utf8_lossy(&first.body);
        let found = find_sound(link, &page).ok_or("aucun son trouvé sur cette page")?;
        let title = if found.title.is_empty() {
            title_from_url(&found.audio)
        } else {
            found.title
        };
        if known(&title) {
            return Err(format!("« {title} » est déjà dans ta bibliothèque"));
        }
        let audio = get(&found.audio, MAX_SOUND_BYTES)?;
        if !is_audio(&audio.content_type, &found.audio) {
            return Err("le lien du son ne mène pas à un fichier audio".into());
        }
        (title, found.audio, audio)
    };
    if known(&title) {
        return Err(format!("« {title} » est déjà dans ta bibliothèque"));
    }
    let ext = extension(&audio_url, &audio.content_type).ok_or("format audio non reconnu")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = save_new(dir, &file_stem(&title), ext, &audio.body)?;
    // Kept only if it really is a sound the app can play.
    match crate::audio::decode::decode(&path.to_string_lossy()) {
        Ok(pcm) => Ok(Downloaded {
            path,
            title,
            duration: pcm.duration(),
        }),
        Err(e) => {
            let _ = fs::remove_file(&path);
            Err(format!("fichier téléchargé illisible : {e}"))
        }
    }
}

// ---------- Finding the sound in a page ----------

#[derive(Debug, PartialEq)]
pub struct Found {
    pub title: String,
    pub audio: String, // absolute https URL
}

/// The sound a page describes: schema.org AudioObject (Voicemod Tuna), else Open Graph
/// og:audio (MyInstants and many others).
pub fn find_sound(page_url: &str, html: &str) -> Option<Found> {
    let same = |u: &str| u.trim_end_matches('/') == page_url.trim_end_matches('/');
    let mut objects = Vec::new();
    for block in scripts(html, "application/ld+json") {
        if let Ok(v) = serde_json::from_str::<Value>(block) {
            audio_objects(&v, &mut objects);
        }
    }
    if let Some((name, content, _)) = objects.iter().find(|o| same(&o.2)).or(objects.first()) {
        return Some(Found {
            title: name.trim().to_string(),
            audio: absolute(page_url, content)?,
        });
    }
    let audio = ["og:audio:secure_url", "og:audio", "og:audio:url"]
        .iter()
        .find_map(|k| meta(html, k))?;
    let title = meta(html, "og:title")
        .map(|t| clean_page_title(&t))
        .unwrap_or_default();
    Some(Found {
        title,
        audio: absolute(page_url, &audio)?,
    })
}

/// (name, contentUrl, url) of every schema.org AudioObject in a JSON-LD value.
fn audio_objects(v: &Value, out: &mut Vec<(String, String, String)>) {
    match v {
        Value::Object(o) => {
            let is_audio = match o.get("@type") {
                Some(Value::String(t)) => t == "AudioObject",
                Some(Value::Array(a)) => a.iter().any(|t| t == "AudioObject"),
                _ => false,
            };
            if is_audio && let Some(content) = o.get("contentUrl").and_then(Value::as_str) {
                let s = |k: &str| {
                    o.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                out.push((s("name"), content.to_string(), s("url")));
            }
            o.values().for_each(|x| audio_objects(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| audio_objects(x, out)),
        _ => {}
    }
}

/// The text of the <script> elements of a given type.
fn scripts<'a>(html: &'a str, kind: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("<script") {
        rest = &rest[i..];
        let Some(tag_end) = rest.find('>') else { break };
        let tag = &rest[..tag_end];
        rest = &rest[tag_end + 1..];
        let Some(end) = rest.find("</script") else {
            break;
        };
        if tag.to_ascii_lowercase().contains(kind) {
            out.push(&rest[..end]);
        }
        rest = &rest[end..];
    }
    out
}

/// Attributes of one tag's text, values with or without quotes.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = tag
        .trim_start_matches('<')
        .trim_start_matches(|c: char| c.is_ascii_alphanumeric());
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '=' || c == '/')
            .unwrap_or(rest.len());
        if name_end == 0 {
            break;
        }
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let Some(after) = rest.strip_prefix('=') else {
            out.push((name, String::new()));
            continue;
        };
        let after = after.trim_start();
        let (value, next) = match after.chars().next() {
            Some(q @ ('"' | '\'')) => match after[1..].find(q) {
                Some(end) => (&after[1..1 + end], &after[end + 2..]),
                None => (&after[1..], ""),
            },
            _ => {
                let end = after.find(char::is_whitespace).unwrap_or(after.len());
                (&after[..end], &after[end..])
            }
        };
        out.push((name, decode_entities(value)));
        rest = next;
    }
    out
}

/// content of <meta property|name="key">.
fn meta(html: &str, key: &str) -> Option<String> {
    html.match_indices("<meta").find_map(|(i, _)| {
        let tag = &html[i..i + html[i..].find('>')?];
        let attrs = attributes(tag);
        let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let named = get("property")
            .or(get("name"))
            .is_some_and(|n| n.eq_ignore_ascii_case(key));
        named
            .then(|| get("content").map(str::to_string))
            .flatten()
            .filter(|c| !c.trim().is_empty())
    })
}

/// "BRUH sound effect! - Sound Button" -> "BRUH sound effect!"; "Title | Site" -> "Title".
fn clean_page_title(t: &str) -> String {
    let t = t.split(" | ").next().unwrap_or(t).trim();
    t.strip_suffix(" - Sound Button")
        .unwrap_or(t)
        .trim()
        .to_string()
}

/// A link found in `page` -> absolute https URL. Plain http is upgraded.
fn absolute(page: &str, link: &str) -> Option<String> {
    let link = link.trim();
    if let Some(rest) = link.strip_prefix("http://") {
        return Some(format!("https://{rest}"));
    }
    if link.starts_with("https://") {
        return Some(link.to_string());
    }
    if let Some(rest) = link.strip_prefix("//") {
        return Some(format!("https://{rest}"));
    }
    let u = parse_https(page).ok()?;
    let origin = if u.port == 443 {
        format!("https://{}", u.host)
    } else {
        format!("https://{}:{}", u.host, u.port)
    };
    if link.starts_with('/') {
        return Some(format!("{origin}{link}"));
    }
    let dir = u.path.split('?').next().unwrap_or("/");
    Some(format!(
        "{origin}{}{link}",
        &dir[..dir.rfind('/').map_or(0, |i| i + 1)]
    ))
}

// ---------- Files ----------

fn url_path(url: &str) -> &str {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    &url[..end]
}

fn url_extension(url: &str) -> Option<&'static str> {
    let path = url_path(url);
    let ext = path
        .rsplit('/')
        .next()?
        .rsplit_once('.')?
        .1
        .to_ascii_lowercase();
    AUDIO_EXT.iter().copied().find(|e| *e == ext)
}

fn is_audio(content_type: &str, url: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    ct.starts_with("audio/")
        || ((ct.is_empty() || ct.starts_with("application/octet-stream"))
            && url_extension(url).is_some())
}

fn extension(url: &str, content_type: &str) -> Option<&'static str> {
    url_extension(url).or_else(|| {
        let ct = content_type.to_ascii_lowercase();
        let ct = ct.split(';').next().unwrap_or("").trim();
        Some(match ct {
            "audio/mpeg" | "audio/mp3" => "mp3",
            "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
            "audio/ogg" | "audio/opus" => "ogg",
            "audio/flac" | "audio/x-flac" => "flac",
            "audio/mp4" | "audio/x-m4a" | "audio/aac" => "m4a",
            "audio/webm" => "webm",
            "audio/aiff" | "audio/x-aiff" => "aiff",
            _ => return None,
        })
    })
}

fn title_from_url(url: &str) -> String {
    let name = url_path(url)
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("son");
    let name = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    crate::model::clean_title(&name.replace("%20", " "))
}

/// A Windows file name from a title: no reserved characters or device names, 80 characters at most.
fn file_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut stem: String = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect();
    stem = stem
        .trim_matches(|c: char| c == '.' || c == ' ')
        .to_string();
    let base = stem.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.as_bytes()[3].is_ascii_digit());
    if stem.is_empty() {
        "son".into()
    } else if reserved {
        format!("son {stem}")
    } else {
        stem
    }
}

/// Writes to a new file, "stem (2).ext" when the name is taken: never over another file.
fn save_new(dir: &Path, stem: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    for n in 1..1000 {
        let name = if n == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let path = dir.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                return f.write_all(bytes).map(|_| path.clone()).map_err(|e| {
                    let _ = fs::remove_file(&path);
                    e.to_string()
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("trop de fichiers du même nom".into())
}

// ---------- HTTPS ----------

#[derive(Debug, PartialEq)]
struct HttpsUrl {
    host: String,
    port: u16,
    path: String, // path and query
}

fn parse_https(url: &str) -> Result<HttpsUrl, String> {
    let url = url.trim();
    let rest = url
        .get(..8)
        .filter(|s| s.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])
        .ok_or("le lien doit commencer par https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.contains('@')
        || authority.starts_with('[')
        || authority
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("adresse de site non prise en charge".into());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "port invalide")?),
        None => (authority, 443),
    };
    if host.is_empty() {
        return Err("adresse de site vide".into());
    }
    let path = url_path(&rest[end..]);
    let query = rest[end..].find('?').map_or("", |q| {
        let tail = &rest[end + q..];
        &tail[..tail.find('#').unwrap_or(tail.len())]
    });
    let path = if path.is_empty() { "/" } else { path };
    Ok(HttpsUrl {
        host: host.to_string(),
        port,
        path: format!("{path}{query}"),
    })
}

pub struct Response {
    content_type: String,
    pub body: Vec<u8>,
}

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn opened(h: *mut c_void) -> Result<Handle, String> {
    if h.is_null() {
        Err(windows::core::Error::from_thread().message())
    } else {
        Ok(Handle(h))
    }
}

/// GET over HTTPS (redirects followed, never down to http). Fails past `max` bytes.
pub fn get(url: &str, max: usize) -> Result<Response, String> {
    let u = parse_https(url)?;
    let (host, path) = (wide(&u.host), wide(&u.path));
    let err = |e: windows::core::Error| e.message();
    unsafe {
        let session = opened(WinHttpOpen(
            w!("SoundpadImproved"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS)
            .map_err(err)?;
        let connection = opened(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), u.port, 0))?;
        let request = opened(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(err)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(err)?;

        let number = |level: u32| {
            let (mut value, mut len) = (0u32, 4u32);
            WinHttpQueryHeaders(
                request.0,
                level | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some((&raw mut value).cast()),
                &mut len,
                std::ptr::null_mut(),
            )
            .ok()
            .map(|_| value)
        };
        let status = number(WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
        if !(200..300).contains(&status) {
            return Err(format!("le site a répondu {status}"));
        }
        if number(WINHTTP_QUERY_CONTENT_LENGTH).is_some_and(|n| n as usize > max) {
            return Err("fichier trop gros".into());
        }
        let mut ct = [0u16; 256];
        let mut len = (ct.len() * 2) as u32;
        let content_type = WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_CONTENT_TYPE,
            PCWSTR::null(),
            Some(ct.as_mut_ptr().cast()),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_or(String::new(), |_| {
            String::from_utf16_lossy(&ct[..len as usize / 2])
        });

        let mut body = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut n = 0u32;
            WinHttpReadData(request.0, buf.as_mut_ptr().cast(), buf.len() as u32, &mut n)
                .map_err(err)?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&buf[..n as usize]);
            if body.len() > max {
                return Err("fichier trop gros".into());
            }
        }
        Ok(Response { content_type, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TUNA: &str = r#"<html><head><meta property="og:title" content="vineboom Meme Sound Effect Button for Soundboard by X | Voicemod Tuna">
<script type="application/ld+json">{"@context":"https://schema.org","@graph":[{"@type":"AudioObject","name":"ai ai ai jake tatoo","url":"https://tuna.voicemod.net/sound/5d4e","contentUrl":"https://us-tuna-sounds-files.voicemod.net/5d4e-1758.mp3"}]}</script>
<script>var x = "<b>";</script></head></html>"#;

    const MYINSTANTS: &str = r#"<meta property="og:title" content="BRUH sound effect! - Sound Button" />
<meta property=og:type content=website>
<meta property="og:audio" content="https://www.myinstants.com/media/sounds/bruh-sound-effect_WstdzdM.mp3" />"#;

    #[test]
    fn finds_the_sound_of_a_tuna_page() {
        assert_eq!(
            find_sound("https://tuna.voicemod.net/sound/5d4e", TUNA),
            Some(Found {
                title: "ai ai ai jake tatoo".into(),
                audio: "https://us-tuna-sounds-files.voicemod.net/5d4e-1758.mp3".into()
            })
        );
    }

    #[test]
    fn finds_the_sound_of_a_myinstants_page() {
        assert_eq!(
            find_sound("https://www.myinstants.com/en/instant/bruh/", MYINSTANTS),
            Some(Found {
                title: "BRUH sound effect!".into(),
                audio: "https://www.myinstants.com/media/sounds/bruh-sound-effect_WstdzdM.mp3"
                    .into()
            })
        );
        assert_eq!(
            find_sound("https://example.com/", "<html><title>rien</title></html>"),
            None
        );
    }

    #[test]
    fn meta_tags_and_relative_links() {
        let html = r#"<meta name="og:audio" content='/media/a&amp;b.mp3'><meta property="og:title" content="Tom &amp; Jerry | Site">"#;
        assert_eq!(meta(html, "og:audio").as_deref(), Some("/media/a&b.mp3"));
        assert_eq!(
            clean_page_title(&meta(html, "og:title").unwrap()),
            "Tom & Jerry"
        );
        assert_eq!(
            absolute("https://s.fr:8443/a/b?x=1", "c.mp3").as_deref(),
            Some("https://s.fr:8443/a/c.mp3")
        );
        assert_eq!(
            absolute("https://s.fr/a/b", "/m/c.mp3").as_deref(),
            Some("https://s.fr/m/c.mp3")
        );
        assert_eq!(
            absolute("https://s.fr/", "http://cdn.fr/c.mp3").as_deref(),
            Some("https://cdn.fr/c.mp3")
        );
    }

    #[test]
    fn only_plain_https_urls() {
        assert_eq!(
            parse_https("HTTPS://tuna.voicemod.net/sound/abc?x=1#top"),
            Ok(HttpsUrl {
                host: "tuna.voicemod.net".into(),
                port: 443,
                path: "/sound/abc?x=1".into()
            })
        );
        assert_eq!(parse_https("https://s.fr:8443").unwrap().path, "/");
        for bad in [
            "http://s.fr/a.mp3",
            "file:///C:/a.mp3",
            "https://user@s.fr/",
            "https://[::1]/",
            "https:///a",
            "https://s.fr:99999/",
        ] {
            assert!(parse_https(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn audio_types_and_names() {
        assert!(is_audio("audio/mpeg", "https://s.fr/x"));
        assert!(is_audio(
            "application/octet-stream",
            "https://s.fr/x.wav?dl=1"
        ));
        assert!(!is_audio("text/html; charset=utf-8", "https://s.fr/x.mp3"));
        assert_eq!(extension("https://s.fr/x", "audio/x-wav"), Some("wav"));
        assert_eq!(extension("https://s.fr/x.OGG", "audio/mpeg"), Some("ogg"));
        assert_eq!(extension("https://s.fr/x", "video/mp4"), None);
        assert_eq!(
            title_from_url("https://s.fr/a/kirby-scream-made-with-Voicemod.mp3?x"),
            "kirby scream"
        );
        assert_eq!(file_stem(r#"a/b:c*"d"#), "a b c d");
        assert_eq!(file_stem("CON"), "son CON");
        assert_eq!(file_stem("com1.x"), "son com1.x");
        assert_eq!(file_stem(" ... "), "son");
        assert_eq!(file_stem(&"é".repeat(200)).chars().count(), 80);
    }

    #[test]
    fn never_overwrites_a_file() {
        let dir = std::env::temp_dir().join(format!("spi-link-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a = save_new(&dir, "son", "mp3", b"1").unwrap();
        let b = save_new(&dir, "son", "mp3", b"2").unwrap();
        assert!(a.ends_with("son.mp3") && b.ends_with("son (2).mp3"));
        assert_eq!(fs::read(&a).unwrap(), b"1");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Real sites: cargo test -- --ignored
    #[test]
    #[ignore]
    fn downloads_from_tuna_and_myinstants() {
        let dir = std::env::temp_dir().join(format!("spi-link-live-{}", std::process::id()));
        for link in [
            "https://tuna.voicemod.net/sound/5fffc26b-ad91-4235-88d3-50f6063eef87",
            "https://www.myinstants.com/en/instant/bruh-sound-effect-26614/",
        ] {
            let d = download(link, &dir, |_| false).unwrap_or_else(|e| panic!("{link} : {e}"));
            println!("{} -> {} ({:.1} s)", link, d.path.display(), d.duration);
            assert!(d.duration > 0.0 && !d.title.is_empty());
        }
        assert!(
            download(
                "https://www.myinstants.com/en/instant/bruh-sound-effect-26614/",
                &dir,
                |_| true
            )
            .is_err()
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
