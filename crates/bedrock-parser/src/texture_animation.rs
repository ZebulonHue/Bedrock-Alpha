//! Read Minecraft's `.png.mcmeta` animation sidecars.
//!
//! An animated block texture is a vertical strip of square frames — `water_still`
//! is 16 wide and 512 tall, so 32 frames — paired with a sidecar naming the
//! playback order and timing:
//!
//! ```json
//! { "animation": { "frametime": 2, "interpolate": true,
//!                  "frames": [0, 1, 2, { "index": 3, "time": 10 }] } }
//! ```
//!
//! `frames` is optional; without it the strip plays straight through. Entries
//! are either a bare index or an index with its own duration, which overrides
//! `frametime` for that frame alone. Durations are in ticks (20 per second).
//!
//! The exporter hands the expanded sequence to the Blender addon, which keys it
//! onto the texture's UV offset. Doing the expansion here keeps the addon from
//! having to re-derive Minecraft's defaulting rules.

use serde::{Deserialize, Serialize};

/// One step of a texture's animation: which row of the strip, and for how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimationStep {
    /// Row of the strip to show, counting from the top.
    pub index: u32,
    /// How long to hold it, in ticks.
    pub ticks: u32,
}

/// A texture's animation, expanded from its `.mcmeta` and strip dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureAnimation {
    /// Rows in the strip. The V axis has to be divided by this to isolate one.
    pub frame_count: u32,
    /// Whether the game cross-fades between steps.
    ///
    /// Reported so the addon can decide what to do; it currently steps.
    pub interpolate: bool,
    /// The playback order, already expanded and defaulted.
    pub steps: Vec<AnimationStep>,
}

impl TextureAnimation {
    /// Total length of one loop, in ticks.
    pub fn total_ticks(&self) -> u32 {
        self.steps.iter().map(|s| s.ticks).sum()
    }
}

/// Parse an `.mcmeta` against the strip it describes.
///
/// `frame_count` comes from the PNG (height / width) rather than the sidecar,
/// which never states it. Returns `None` when the sidecar carries no
/// `animation` block — some describe only GUI scaling — or when the strip is a
/// single frame, since there is then nothing to animate.
pub fn parse_mcmeta(mcmeta: &[u8], frame_count: u32) -> Option<TextureAnimation> {
    if frame_count < 2 {
        return None;
    }
    let root: serde_json::Value = serde_json::from_slice(mcmeta).ok()?;
    let animation = root.get("animation")?;

    // Minecraft's default is one tick per frame. A zero would stall the
    // animation outright, so treat it as the default too.
    let frametime = animation
        .get("frametime")
        .and_then(serde_json::Value::as_u64)
        .filter(|t| *t > 0)
        .unwrap_or(1) as u32;
    let interpolate = animation
        .get("interpolate")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let steps = match animation.get("frames").and_then(serde_json::Value::as_array) {
        Some(frames) => frames
            .iter()
            .filter_map(|frame| {
                let (index, ticks) = match frame {
                    serde_json::Value::Number(n) => (n.as_u64()?, u64::from(frametime)),
                    serde_json::Value::Object(_) => (
                        frame.get("index").and_then(serde_json::Value::as_u64)?,
                        frame
                            .get("time")
                            .and_then(serde_json::Value::as_u64)
                            .filter(|t| *t > 0)
                            .unwrap_or(u64::from(frametime)),
                    ),
                    _ => return None,
                };
                // A sidecar that names a row the strip does not have would
                // sample past the end of the image; drop it rather than
                // showing the wrong frame.
                (index < u64::from(frame_count)).then_some(AnimationStep {
                    index: index as u32,
                    ticks: ticks as u32,
                })
            })
            .collect(),
        None => (0..frame_count)
            .map(|index| AnimationStep {
                index,
                ticks: frametime,
            })
            .collect(),
    };

    let steps: Vec<AnimationStep> = steps;
    if steps.is_empty() {
        return None;
    }
    Some(TextureAnimation {
        frame_count,
        interpolate,
        steps,
    })
}

/// Rows in a texture strip, or `None` when it is a plain square image.
pub fn strip_frame_count(width: u32, height: u32) -> Option<u32> {
    if width == 0 || height <= width || height % width != 0 {
        return None;
    }
    Some(height / width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_straight_through_without_a_frames_list() {
        let anim = parse_mcmeta(br#"{"animation":{}}"#, 4).unwrap();
        assert_eq!(
            anim.steps,
            vec![
                AnimationStep { index: 0, ticks: 1 },
                AnimationStep { index: 1, ticks: 1 },
                AnimationStep { index: 2, ticks: 1 },
                AnimationStep { index: 3, ticks: 1 },
            ]
        );
        assert_eq!(anim.total_ticks(), 4);
    }

    #[test]
    fn frametime_applies_to_every_step() {
        let anim = parse_mcmeta(br#"{"animation":{"frametime":3}}"#, 2).unwrap();
        assert!(anim.steps.iter().all(|s| s.ticks == 3));
        assert_eq!(anim.total_ticks(), 6);
    }

    #[test]
    fn per_frame_time_overrides_frametime() {
        // Magma and prismarine hold single frames far longer than the rest.
        let anim = parse_mcmeta(
            br#"{"animation":{"frametime":2,"frames":[0,{"index":1,"time":10}]}}"#,
            2,
        )
        .unwrap();
        assert_eq!(
            anim.steps,
            vec![
                AnimationStep { index: 0, ticks: 2 },
                AnimationStep {
                    index: 1,
                    ticks: 10
                },
            ]
        );
    }

    #[test]
    fn an_explicit_order_is_kept_verbatim() {
        // Some textures ping-pong rather than looping, and repeat rows.
        let anim = parse_mcmeta(br#"{"animation":{"frames":[0,1,2,1]}}"#, 3).unwrap();
        let order: Vec<u32> = anim.steps.iter().map(|s| s.index).collect();
        assert_eq!(order, vec![0, 1, 2, 1]);
    }

    #[test]
    fn rows_past_the_end_of_the_strip_are_dropped() {
        let anim = parse_mcmeta(br#"{"animation":{"frames":[0,9]}}"#, 2).unwrap();
        assert_eq!(anim.steps.len(), 1);
    }

    #[test]
    fn a_sidecar_without_an_animation_block_is_not_animated() {
        assert!(parse_mcmeta(br#"{"gui":{"scaling":{}}}"#, 4).is_none());
    }

    #[test]
    fn a_square_texture_is_never_a_strip() {
        assert_eq!(strip_frame_count(16, 16), None);
        assert_eq!(strip_frame_count(16, 512), Some(32));
        // Odd sizes are some other kind of atlas, not a flipbook.
        assert_eq!(strip_frame_count(16, 24), None);
    }
}
