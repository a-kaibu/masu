//! Share groups (review of 6852be1, stage 4): random contexts made alone or sharing another's
//! objects (wglCreateContextAttribsARB's share context, wglShareLists), names made on them and
//! looked up from each, against a model of one group per context. A context sees exactly the
//! objects of its own group; a group goes with its last context.

use super::*;
use crate::d3d9::trace_tests::SERIAL;
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum Op {
    /// a context sharing context .0's objects (None: of its own)
    Make(Option<u8>),
    /// a texture and a buffer made on context .0
    Name(u8),
    /// wglShareLists(.0, .1)
    Share(u8, u8),
    /// wglDeleteContext(.0)
    Delete(u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        proptest::option::of(any::<u8>()).prop_map(Op::Make),
        any::<u8>().prop_map(Op::Name),
        (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Op::Share(a, b)),
        any::<u8>().prop_map(Op::Delete),
    ]
}

/// (texture, buffer) names made on `ctx`.
fn names_on(ctx: u32) -> (u32, u32) {
    vngl_make_current(ctx, 0, 0, 0);
    let (mut t, mut b) = (0u32, 0u32);
    unsafe {
        glGenTextures(1, &mut t);
        glGenBuffers(1, &mut b);
    }
    glBindTexture(GL_TEXTURE_2D, t);
    glBindBuffer(GL_ARRAY_BUFFER, b);
    vngl_make_current(0, 0, 0, 0);
    (t, b)
}

/// Whether `ctx` sees the texture and the buffer.
fn sees(ctx: u32, (t, b): (u32, u32)) -> (bool, bool) {
    vngl_make_current(ctx, 0, 0, 0);
    let r = (glIsTexture(t) == 1, glIsBuffer(b) == 1);
    vngl_make_current(0, 0, 0, 0);
    r
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn a_context_sees_its_groups_objects(ops in proptest::collection::vec(op(), 1..40)) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        *GL.lock().unwrap_or_else(|e| e.into_inner()) = Gl::new();
        // the model: (context, its group), the names made and the group they were made in
        let mut ctxs: Vec<(u32, usize)> = Vec::new();
        let mut made: Vec<((u32, u32), usize)> = Vec::new();
        let mut next_group = 0;
        let pick = |ctxs: &[(u32, usize)], i: u8| (!ctxs.is_empty()).then(|| usize::from(i) % ctxs.len());
        for o in &ops {
            match *o {
                Op::Make(share) => {
                    let with = share.and_then(|i| pick(&ctxs, i));
                    let id = vngl_context_create(with.map_or(0, |k| ctxs[k].0));
                    prop_assert_ne!(id, 0);
                    let group = with.map_or_else(|| { next_group += 1; next_group }, |k| ctxs[k].1);
                    ctxs.push((id, group));
                }
                Op::Name(i) => {
                    let Some(k) = pick(&ctxs, i) else { continue };
                    made.push((names_on(ctxs[k].0), ctxs[k].1));
                }
                Op::Share(a, b) => {
                    let (Some(a), Some(b)) = (pick(&ctxs, a), pick(&ctxs, b)) else { continue };
                    let (ga, gb) = (ctxs[a].1, ctxs[b].1);
                    let alone = ctxs.iter().filter(|c| c.1 == gb).count() == 1;
                    let empty = !made.iter().any(|m| m.1 == gb);
                    let want = ga == gb || (alone && empty);
                    prop_assert_eq!(vngl_share_lists(ctxs[a].0, ctxs[b].0) == 1, want, "{:?}", o);
                    if want {
                        ctxs[b].1 = ga;
                    }
                }
                Op::Delete(i) => {
                    let Some(k) = pick(&ctxs, i) else { continue };
                    let (id, group) = ctxs.remove(k);
                    vngl_context_destroy(id);
                    if !ctxs.iter().any(|c| c.1 == group) {
                        made.retain(|m| m.1 != group);
                    }
                }
            }
            for &(ctx, group) in &ctxs {
                // (each group names from 1: a name of another group may be one of its own too)
                for &((t, b), g) in &made {
                    let want = (made.iter().any(|m| m.1 == group && m.0.0 == t), made.iter().any(|m| m.1 == group && m.0.1 == b));
                    prop_assert_eq!(sees(ctx, (t, b)), want, "context {} of group {}, names {:?} of group {} after {:?}", ctx, group, (t, b), g, o);
                }
            }
        }
        // (a group goes with its last context: none left, no objects left)
        for &(id, _) in &ctxs {
            vngl_context_destroy(id);
        }
        let g = GL.lock().unwrap_or_else(|e| e.into_inner());
        prop_assert!(g.groups.is_empty(), "groups left: {}", g.groups.len());
        prop_assert!(g.objs.textures.is_empty() && g.objs.buffers.is_empty());
    }
}

/// wglCreateContextAttribsARB with a share context that is no context makes none.
#[test]
fn sharing_with_no_context_makes_none() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    *GL.lock().unwrap_or_else(|e| e.into_inner()) = Gl::new();
    assert_eq!(vngl_context_create(77), 0);
    let a = vngl_context_create(0);
    assert_eq!(vngl_share_lists(a, 77), 0);
    vngl_context_destroy(a);
}
