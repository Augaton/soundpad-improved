// Import of Soundpad's soundlist.spl: sounds, hotkeys and (nested) categories.
use std::collections::HashSet;

use crate::keys;
use crate::model::{
    Category, Library, Sound, clean_title, new_id, parse_duration, title_from_path,
};

#[derive(Debug, PartialEq)]
pub struct ParsedSound {
    pub path: String,
    pub title: String,
    pub hotkey: Option<String>,
    pub duration: Option<f64>,
}

#[derive(Debug, PartialEq)]
pub struct ParsedCategory {
    pub name: String,
    pub indexes: Vec<usize>, // positions in `sounds`
}

#[derive(Debug, PartialEq, Default)]
pub struct Parsed {
    pub sounds: Vec<ParsedSound>,
    pub categories: Vec<ParsedCategory>,
}

pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').filter(|&end| end > 1).and_then(|end| {
            let e = &rest[1..end];
            let c = match e {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => e
                    .strip_prefix("#x")
                    .or_else(|| e.strip_prefix("#X"))
                    .map(|h| u32::from_str_radix(h, 16).ok())
                    .unwrap_or_else(|| e.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

struct Tag {
    close: bool,
    name: String,
    attrs: Vec<(String, String)>,
    self_closing: bool,
}

impl Tag {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Start and end tags with their attributes. Enough for Soundpad's flat, attribute-only XML.
fn tags(text: &str) -> Vec<Tag> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        // The closing '>', outside quoted attribute values.
        let mut quote = None;
        let end = rest.char_indices().find_map(|(i, c)| {
            match (quote, c) {
                (None, '"' | '\'') => quote = Some(c),
                (Some(q), _) if c == q => quote = None,
                (None, '>') => return Some(i),
                _ => {}
            }
            None
        });
        let Some(end) = end else { break };
        let body = &rest[..end];
        rest = &rest[end + 1..];
        if body.starts_with(['?', '!']) {
            continue;
        }
        let (close, body) = match body.strip_prefix('/') {
            Some(b) => (true, b),
            None => (false, body),
        };
        let (self_closing, body) = match body.trim_end().strip_suffix('/') {
            Some(b) => (true, b),
            None => (false, body),
        };
        let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
        let name = body[..name_end].to_string();
        let mut attrs = Vec::new();
        let mut a = &body[name_end..];
        while let Some(eq) = a.find('=') {
            let key = a[..eq].trim().to_string();
            let after = a[eq + 1..].trim_start();
            let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                break;
            };
            let Some(close_q) = after[1..].find(q) else {
                break;
            };
            attrs.push((key, decode_entities(&after[1..1 + close_q])));
            a = &after[close_q + 2..];
        }
        if !name.is_empty() {
            out.push(Tag {
                close,
                name,
                attrs,
                self_closing,
            });
        }
    }
    out
}

/// Soundpad soundlist.spl -> sounds and categories. Category entries reference sounds by
/// 1-based position in the list.
pub fn parse_soundpad_list(text: &str) -> Parsed {
    let mut sounds = Vec::new();
    let mut categories: Vec<(String, Vec<i64>)> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut cat_stack: Vec<Option<usize>> = Vec::new(); // index in `categories`
    for t in tags(text) {
        if t.close {
            if stack.pop().as_deref() == Some("Category") {
                cat_stack.pop();
            }
            continue;
        }
        let parent = stack.last().map(String::as_str);
        if t.name == "Sound"
            && parent == Some("Soundlist")
            && t.attr("url").is_some_and(|u| !u.is_empty())
        {
            let url = t.attr("url").unwrap_or_default();
            let key: i64 = t.attr("key").and_then(|k| k.parse().ok()).unwrap_or(0);
            let mods: u32 = t
                .attr("keyModifiers")
                .and_then(|k| k.parse().ok())
                .unwrap_or(0);
            let title = t
                .attr("title")
                .filter(|s| !s.is_empty())
                .map_or_else(|| title_from_path(url), str::to_string);
            sounds.push(ParsedSound {
                path: url.to_string(),
                title: clean_title(&title),
                hotkey: if key > 0 {
                    keys::vk_to_accelerator(key as u32, mods)
                } else {
                    None
                },
                duration: t.attr("duration").and_then(parse_duration),
            });
        } else if t.name == "Sound" && parent == Some("Category") {
            if let Some(Some(c)) = cat_stack.last() {
                categories[*c]
                    .1
                    .push(t.attr("id").and_then(|i| i.parse().ok()).unwrap_or(0));
            }
        } else if t.name == "Category" {
            // Built-in categories ("All sounds") carry a type and no name.
            let named = t
                .attr("type")
                .is_none()
                .then(|| t.attr("name"))
                .flatten()
                .filter(|n| !n.is_empty());
            let cat = named.map(|name| {
                let mut path: Vec<&str> = cat_stack
                    .iter()
                    .flatten()
                    .map(|&c| categories[c].0.as_str())
                    .collect();
                path.push(name);
                categories.push((path.join(" / "), Vec::new()));
                categories.len() - 1
            });
            if !t.self_closing {
                stack.push("Category".into());
                cat_stack.push(cat);
            }
            continue;
        }
        if !t.self_closing {
            stack.push(t.name);
        }
    }
    let n = sounds.len() as i64;
    Parsed {
        categories: categories
            .into_iter()
            .map(|(name, ids)| ParsedCategory {
                name,
                indexes: ids
                    .into_iter()
                    .filter(|&i| i >= 1 && i <= n)
                    .map(|i| (i - 1) as usize)
                    .collect(),
            })
            .collect(),
        sounds,
    }
}

/// Adds an imported list to the library. Sounds already present (same path, any case) are
/// reused, never duplicated; a hotkey is only taken over when no other sound uses it.
/// Returns (added, skipped).
pub fn merge_import(library: &mut Library, parsed: &Parsed) -> (usize, usize) {
    let mut used: HashSet<String> = library
        .sounds
        .iter()
        .filter_map(|s| s.hotkey.clone())
        .collect();
    let mut added = 0;
    let ids: Vec<String> = parsed
        .sounds
        .iter()
        .map(|p| {
            if let Some(existing) = library
                .sounds
                .iter()
                .find(|s| s.path.to_lowercase() == p.path.to_lowercase())
            {
                return existing.id.clone();
            }
            let hotkey = p.hotkey.clone().filter(|h| !used.contains(h));
            if let Some(h) = &hotkey {
                used.insert(h.clone());
            }
            let mut sound = Sound::new(&p.path);
            sound.title = p.title.clone();
            sound.hotkey = hotkey;
            sound.duration = p.duration;
            let id = sound.id.clone();
            library.sounds.push(sound);
            added += 1;
            id
        })
        .collect();
    for pc in &parsed.categories {
        let cat = match library.categories.iter().position(|c| c.name == pc.name) {
            Some(i) => &mut library.categories[i],
            None => {
                library.categories.push(Category {
                    id: new_id(),
                    name: pc.name.clone(),
                    sound_ids: Vec::new(),
                    hotkey: None,
                });
                library.categories.last_mut().unwrap()
            }
        };
        for &i in &pc.indexes {
            if !cat.sound_ids.contains(&ids[i]) {
                cat.sound_ids.push(ids[i].clone());
            }
        }
    }
    (added, parsed.sounds.len() - added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::normalize_library;
    use serde_json::json;

    const SPL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Soundlist>
  <Sound hash="1" url="C:\soundpad\ba dum tss.mp3" artist="" title="ba dum tss" duration="0:02" addedOn="2026-03-09"/>
  <Sound hash="2" url="C:\soundpad\kirby-made-with-Voicemod.mp3" artist="" title="kirby-made-with-Voicemod" duration="0:06"/>
  <Sound hash="3" url="C:\soundpad\Tom &amp; Jerry.mp3" artist="" title="Tom &amp; Jerry &#233;t&#xE9;" duration="3:23" keyModifiers="0" key="19"/>
  <Categories>
    <Category type="1" icon="stock_icon_cat_list" hidden="true"/>
    <Category name="Mes sons" icon="stock_icon_cat_home">
      <Sound id="3"/>
      <Sound id="1"/>
      <Sound id="99"/>
      <Category name="Memes">
        <Sound id="2"/>
      </Category>
    </Category>
  </Categories>
  <Hotbar>
    <HotbarPage/>
  </Hotbar>
</Soundlist>"#;

    #[test]
    fn reads_sounds_hotkeys_and_nested_categories() {
        let parsed = parse_soundpad_list(SPL);
        let s = |path: &str, title: &str, hotkey: Option<&str>, duration: f64| ParsedSound {
            path: path.into(),
            title: title.into(),
            hotkey: hotkey.map(Into::into),
            duration: Some(duration),
        };
        assert_eq!(
            parsed.sounds,
            vec![
                s("C:\\soundpad\\ba dum tss.mp3", "ba dum tss", None, 2.0),
                s(
                    "C:\\soundpad\\kirby-made-with-Voicemod.mp3",
                    "kirby",
                    None,
                    6.0
                ),
                s(
                    "C:\\soundpad\\Tom & Jerry.mp3",
                    "Tom & Jerry été",
                    Some("Pause"),
                    203.0
                ),
            ]
        );
        assert_eq!(
            parsed.categories,
            vec![
                ParsedCategory {
                    name: "Mes sons".into(),
                    indexes: vec![2, 0]
                },
                ParsedCategory {
                    name: "Mes sons / Memes".into(),
                    indexes: vec![1]
                },
            ]
        );
    }

    #[test]
    fn merge_never_duplicates_sounds_categories_or_hotkeys() {
        let mut lib = normalize_library(&json!({
            "sounds": [
                { "id": "mine", "path": "c:\\SOUNDPAD\\ba dum tss.mp3" },
                { "id": "k", "path": "C:\\k.mp3", "hotkey": "Pause" },
            ],
        }));
        let parsed = parse_soundpad_list(SPL);
        assert_eq!(merge_import(&mut lib, &parsed), (2, 1));
        assert_eq!(lib.sounds.len(), 4);
        let tom = lib
            .sounds
            .iter()
            .find(|s| s.title == "Tom & Jerry été")
            .unwrap()
            .clone();
        assert_eq!(tom.hotkey, None, "Pause already belongs to another sound");
        let mes_sons = |lib: &Library| {
            lib.categories
                .iter()
                .find(|c| c.name == "Mes sons")
                .unwrap()
                .sound_ids
                .clone()
        };
        assert_eq!(mes_sons(&lib), vec![tom.id.clone(), "mine".to_string()]);

        assert_eq!(merge_import(&mut lib, &parsed), (0, 3));
        assert_eq!(lib.categories.len(), 2);
        assert_eq!(mes_sons(&lib), vec![tom.id, "mine".to_string()]);
    }

    #[test]
    fn entities() {
        assert_eq!(
            decode_entities("a &amp; b &lt;&#233;&#xe9;&bogus; &"),
            "a & b <éé&bogus; &"
        );
        let parsed = parse_soundpad_list(
            r#"<Soundlist><Sound url="C:\a>b.mp3" title="a > b"/></Soundlist>"#,
        );
        assert_eq!(parsed.sounds[0].title, "a > b");
    }
}
