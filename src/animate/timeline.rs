// prism-runtime/src/animate/timeline.rs
//
// Animation timeline — manages active animations and advances them each frame.

use crate::prd::document::PrdDocument;
use crate::prd::animation::{AnimationDef, AnimationDirection, AnimationFillMode};
use crate::prd::node::NodeId;
use crate::animate::easing::evaluate;
use crate::animate::interpolate::{interpolate, apply_animated_property};

/// A running animation instance.
#[derive(Debug, Clone)]
struct ActiveAnimation {
    /// The animation definition index in the document.
    def_index: u32,
    /// The target node ID.
    node_id: NodeId,
    /// Elapsed time in milliseconds.
    elapsed_ms: f32,
    /// Current iteration count.
    iteration: f32,
    /// Is this animation finished?
    finished: bool,
}

/// Manages all active animations.
pub struct AnimationTimeline {
    active: Vec<ActiveAnimation>,
}

impl AnimationTimeline {
    pub fn new() -> Self {
        Self {
            active: Vec::new(),
        }
    }

    /// Start an animation on a node.
    pub fn start(&mut self, def_index: u32, node_id: NodeId) {
        self.active.push(ActiveAnimation {
            def_index,
            node_id,
            elapsed_ms: 0.0,
            iteration: 0.0,
            finished: false,
        });
    }

    /// Start all animations configured on nodes in the document.
    pub fn start_all(&mut self, doc: &PrdDocument) {
        for node in &doc.nodes {
            for &anim_idx in &node.animations {
                self.start(anim_idx, node.id);
            }
        }
    }

    /// Advance all active animations by `dt` seconds.
    /// Modifies node styles in the document to reflect current animation state.
    pub fn advance(&mut self, doc: &mut PrdDocument, dt: f32) {
        let dt_ms = dt * 1000.0;

        for anim in &mut self.active {
            if anim.finished {
                continue;
            }

            let def = match doc.animations.get(anim.def_index as usize) {
                Some(d) => d.clone(),
                None => {
                    anim.finished = true;
                    continue;
                }
            };

            // Handle delay.
            if anim.elapsed_ms < def.delay_ms {
                anim.elapsed_ms += dt_ms;
                if anim.elapsed_ms < def.delay_ms {
                    // Apply fill-mode backwards if needed.
                    if matches!(def.fill_mode, AnimationFillMode::Backwards | AnimationFillMode::Both) {
                        apply_keyframe_at(doc, &def, anim.node_id, 0.0);
                    }
                    continue;
                }
            }

            let active_time = anim.elapsed_ms - def.delay_ms;
            let duration = def.duration_ms.max(0.001); // Avoid division by zero.

            // Calculate current iteration and progress.
            let raw_iteration = active_time / duration;
            let current_iteration = raw_iteration.floor();

            if !def.iteration_count.is_infinite() && raw_iteration >= def.iteration_count {
                anim.finished = true;
                // Apply fill-mode forwards.
                if matches!(def.fill_mode, AnimationFillMode::Forwards | AnimationFillMode::Both) {
                    apply_keyframe_at(doc, &def, anim.node_id, 1.0);
                }
                continue;
            }

            let mut progress = raw_iteration - current_iteration; // 0.0–1.0 within current iteration.

            // Handle direction.
            let is_reversed = match def.direction {
                AnimationDirection::Normal => false,
                AnimationDirection::Reverse => true,
                AnimationDirection::Alternate => current_iteration as u32 % 2 == 1,
                AnimationDirection::AlternateReverse => current_iteration as u32 % 2 == 0,
            };

            if is_reversed {
                progress = 1.0 - progress;
            }

            apply_keyframe_at(doc, &def, anim.node_id, progress);

            anim.elapsed_ms += dt_ms;
            anim.iteration = current_iteration;
        }

        // Remove finished animations.
        self.active.retain(|a| !a.finished);
    }

    /// Check if there are any active animations.
    pub fn has_active(&self) -> bool {
        !self.active.is_empty()
    }
}

/// Apply interpolated keyframe properties to a node at the given progress (0.0–1.0).
fn apply_keyframe_at(
    doc: &mut PrdDocument,
    def: &AnimationDef,
    node_id: NodeId,
    progress: f32,
) {
    if def.keyframes.len() < 2 {
        return;
    }

    // Find the two keyframes surrounding `progress`.
    let mut from_idx = 0;
    let mut to_idx = 1;
    for i in 0..def.keyframes.len() - 1 {
        if def.keyframes[i].offset <= progress && def.keyframes[i + 1].offset >= progress {
            from_idx = i;
            to_idx = i + 1;
            break;
        }
    }

    let from_kf = &def.keyframes[from_idx];
    let to_kf = &def.keyframes[to_idx];

    // Normalize progress within this keyframe segment.
    let segment_duration = to_kf.offset - from_kf.offset;
    let local_t = if segment_duration > 0.0 {
        ((progress - from_kf.offset) / segment_duration).clamp(0.0, 1.0)
    } else {
        1.0
    };

    // Apply easing.
    let eased_t = evaluate(&from_kf.easing, local_t);

    // Interpolate and apply each property.
    let node = match doc.get_node_mut(node_id) {
        Some(n) => n,
        None => return,
    };

    for from_prop in &from_kf.properties {
        // Find the matching property in the 'to' keyframe.
        if let Some(to_prop) = to_kf.properties.iter().find(|p| p.name() == from_prop.name()) {
            let interpolated = interpolate(from_prop, to_prop, eased_t);
            apply_animated_property(&mut node.style, &interpolated);
        }
    }
}

