//! Sound effects mpv applies on the fly, the audio delay remembered per dub, and the audio
//! tracks of the playing file.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Effects the user switches on one by one; all off plays the source as it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioFx {
    /// Evens out loudness: whispers and explosions, one dub and another.
    pub even: bool,
    /// Lifts voices out of the music.
    pub voice: bool,
    /// Squashes loud moments so nothing startles at low volume.
    pub night: bool,
    /// Removes the hiss of a home-recorded dub.
    pub denoise: bool,
    /// Blends the channels a little, easier on the ears in headphones.
    pub headphones: bool,
}

impl AudioFx {
    pub fn from_value(v: Option<Value>) -> Self {
        v.and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }
}

/// mpv's `af` value: one labelled lavfi graph, or empty for none. Noise goes before anything
/// raises the level, and a limiter closes the chain since several effects add gain.
pub fn filter_chain(fx: &AudioFx) -> String {
    let steps = [
        (fx.denoise, "afftdn=nr=10:nf=-50:tn=1"),
        (fx.voice, "dialoguenhance"),
        (
            fx.night,
            "acompressor=threshold=0.05:ratio=6:attack=5:release=250:makeup=4",
        ),
        (fx.even, "dynaudnorm=f=250:g=15:p=0.9:m=8"),
        (fx.headphones, "crossfeed=strength=0.3"),
    ];
    let graph: Vec<&str> = steps
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, f)| *f)
        .collect();
    if graph.is_empty() {
        return String::new();
    }
    format!(
        "@fx:lavfi=[{},alimiter=limit=0.95:level=disabled]",
        graph.join(",")
    )
}

/// mpv's `audio-delay` in seconds: within five seconds either way, in 10 ms steps.
pub fn clamp_delay(v: f64) -> f64 {
    if !v.is_finite() {
        return 0.0;
    }
    (v.clamp(-5.0, 5.0) * 100.0).round() / 100.0
}

/// The delay saved for a dub; `saved` maps dub keys to seconds.
pub fn delay_for(saved: Option<&Value>, dub: &str) -> f64 {
    saved
        .and_then(|s| s.get(dub))
        .and_then(Value::as_f64)
        .map_or(0.0, clamp_delay)
}

/// `saved` with the delay of a dub changed; a dub back in sync is dropped.
pub fn remember_delay(saved: Option<Value>, dub: &str, delay: f64) -> Value {
    let mut map = match saved {
        Some(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    let delay = clamp_delay(delay);
    if delay == 0.0 {
        map.remove(dub);
    } else {
        map.insert(dub.to_string(), delay.into());
    }
    Value::Object(map)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AudioTrack {
    pub id: i64,
    pub label: String,
}

fn language(code: &str) -> String {
    match code {
        "rus" | "ru" => "Русский".into(),
        "jpn" | "ja" => "Японский".into(),
        "eng" | "en" => "Английский".into(),
        "ukr" | "uk" => "Украинский".into(),
        "chi" | "zho" | "zh" => "Китайский".into(),
        "kor" | "ko" => "Корейский".into(),
        other => other.to_uppercase(),
    }
}

/// Audio tracks of mpv's `track-list` worth choosing between. Tracks without a name or
/// language, and repeated names, are copies of one track (a master playlist lists the audio
/// once per quality), so they are no choice at all.
pub fn audio_tracks(list: &Value) -> Vec<AudioTrack> {
    let mut out: Vec<AudioTrack> = Vec::new();
    for t in list.as_array().into_iter().flatten() {
        if t["type"] != "audio" {
            continue;
        }
        let Some(id) = t["id"].as_i64() else { continue };
        let label = match (t["title"].as_str(), t["lang"].as_str()) {
            (Some(title), _) if !title.trim().is_empty() => title.trim().to_string(),
            (_, Some(lang)) if !lang.is_empty() => language(lang),
            _ => continue,
        };
        if !out.iter().any(|x| x.label == label) {
            out.push(AudioTrack { id, label });
        }
    }
    if out.len() < 2 {
        out.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LIMITER: &str = "alimiter=limit=0.95:level=disabled";

    #[test]
    fn no_effects_leave_the_sound_alone() {
        assert_eq!(filter_chain(&AudioFx::default()), "");
    }

    #[test]
    fn effects_run_in_a_fixed_order_and_a_limiter_closes_the_chain() {
        let all = AudioFx {
            even: true,
            voice: true,
            night: true,
            denoise: true,
            headphones: true,
        };
        assert_eq!(
            filter_chain(&all),
            format!(
                "@fx:lavfi=[afftdn=nr=10:nf=-50:tn=1,dialoguenhance,\
                 acompressor=threshold=0.05:ratio=6:attack=5:release=250:makeup=4,\
                 dynaudnorm=f=250:g=15:p=0.9:m=8,crossfeed=strength=0.3,{LIMITER}]"
            )
        );
        let voice = AudioFx {
            voice: true,
            ..AudioFx::default()
        };
        assert_eq!(
            filter_chain(&voice),
            format!("@fx:lavfi=[dialoguenhance,{LIMITER}]")
        );
    }

    #[test]
    fn saved_effects_parse_leniently() {
        assert_eq!(
            AudioFx::from_value(Some(json!({ "even": true, "unknown": 1 }))),
            AudioFx {
                even: true,
                ..AudioFx::default()
            }
        );
        assert_eq!(AudioFx::from_value(Some(json!("loud"))), AudioFx::default());
        assert_eq!(AudioFx::from_value(None), AudioFx::default());
    }

    #[test]
    fn delay_is_remembered_per_dub() {
        let saved = remember_delay(None, "anidub", 0.15);
        let saved = remember_delay(Some(saved), "shiza", -0.3);
        assert_eq!(delay_for(Some(&saved), "anidub"), 0.15);
        assert_eq!(delay_for(Some(&saved), "shiza"), -0.3);
        assert_eq!(delay_for(Some(&saved), "jam"), 0.0);
        assert_eq!(delay_for(None, "anidub"), 0.0);
        let reset = remember_delay(Some(saved), "anidub", 0.0);
        assert_eq!(reset, json!({ "shiza": -0.3 }));
    }

    #[test]
    fn delay_stays_within_five_seconds_in_ten_millisecond_steps() {
        assert_eq!(clamp_delay(0.123), 0.12);
        assert_eq!(clamp_delay(-0.051), -0.05);
        assert_eq!(clamp_delay(9.0), 5.0);
        assert_eq!(clamp_delay(-9.0), -5.0);
        assert_eq!(clamp_delay(f64::NAN), 0.0);
    }

    #[test]
    fn audio_tracks_are_listed_by_title_or_language() {
        let list = json!([
            { "id": 1, "type": "video" },
            { "id": 1, "type": "audio", "lang": "jpn" },
            { "id": 2, "type": "audio", "lang": "rus", "title": "AniLibria" },
            { "id": 3, "type": "audio", "lang": "eng" },
            { "id": 1, "type": "sub", "lang": "rus" }
        ]);
        assert_eq!(
            audio_tracks(&list),
            vec![
                AudioTrack {
                    id: 1,
                    label: "Японский".into()
                },
                AudioTrack {
                    id: 2,
                    label: "AniLibria".into()
                },
                AudioTrack {
                    id: 3,
                    label: "Английский".into()
                },
            ]
        );
    }

    async fn next_af(
        events: &mut tokio::sync::mpsc::UnboundedReceiver<crate::player::ipc::MpvEvent>,
    ) -> Value {
        loop {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
                .await
                .expect("timed out")
                .expect("closed");
            if let crate::player::ipc::MpvEvent::Property { name, data } = ev {
                if name == "af" {
                    return data;
                }
            }
        }
    }

    /// The chain goes to a playing mpv over IPC, as the app sends it, and can be taken off.
    #[tokio::test]
    #[ignore = "needs mpv: scripts/fetch-mpv.ps1 on Windows, the system mpv elsewhere"]
    async fn real_mpv_takes_the_chain_while_playing() {
        use crate::player::ipc::testing::{ipc_path, mpv_path};
        use crate::player::ipc::Ipc;
        let mpv = mpv_path();
        let pipe = ipc_path("audio");
        let mut child = std::process::Command::new(mpv)
            .args([
                "--idle=yes",
                "--no-config",
                "--terminal=no",
                "--vo=null",
                "--ao=null",
            ])
            .arg(format!("--input-ipc-server={pipe}"))
            .spawn()
            .unwrap();
        let (ipc, mut events) = Ipc::connect(&pipe, std::time::Duration::from_secs(10))
            .await
            .unwrap();
        ipc.observe(1, "af").await.unwrap();
        ipc.command(json!([
            "loadfile",
            "av://lavfi:sine=frequency=440:duration=8,aformat=channel_layouts=stereo"
        ]))
        .await
        .unwrap();
        let all = AudioFx {
            even: true,
            voice: true,
            night: true,
            denoise: true,
            headphones: true,
        };
        ipc.command(json!(["set_property", "af", filter_chain(&all)]))
            .await
            .unwrap();
        let mut af = next_af(&mut events).await;
        while af == json!([]) {
            af = next_af(&mut events).await;
        }
        assert_eq!(af[0]["label"], "fx", "{af}");
        ipc.command(json!(["set_property", "af", ""]))
            .await
            .unwrap();
        assert_eq!(next_af(&mut events).await, json!([]));
        ipc.command(json!(["quit"])).await.unwrap();
        let _ = child.wait();
    }

    #[test]
    fn copies_of_one_track_are_not_a_choice() {
        // A master playlist lists the same audio once per quality, without names.
        let variants = json!([
            { "id": 1, "type": "audio" },
            { "id": 2, "type": "audio" },
            { "id": 3, "type": "audio", "lang": "rus" },
            { "id": 4, "type": "audio", "lang": "rus" }
        ]);
        assert!(audio_tracks(&variants).is_empty());
        let single = json!([{ "id": 1, "type": "audio", "lang": "rus" }]);
        assert!(audio_tracks(&single).is_empty());
        assert!(audio_tracks(&json!(null)).is_empty());
    }
}
