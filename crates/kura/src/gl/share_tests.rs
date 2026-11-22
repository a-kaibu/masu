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
    /// a framebuffer made on context .0, by glGenFramebuffers or (.1) glGenFramebuffersEXT
    Fbo(u8, bool),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        proptest::option::of(any::<u8>()).prop_map(Op::Make),
        any::<u8>().prop_map(Op::Name),
        (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Op::Share(a, b)),
        any::<u8>().prop_map(Op::Delete),
        (any::<u8>(), any::<bool>()).prop_map(|(c, e)| Op::Fbo(c, e)),
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

/// A framebuffer made and bound on `ctx` (EXT_framebuffer_object's with `ext`).
fn fbo_on(ctx: u32, ext: bool) -> u32 {
    vngl_make_current(ctx, 0, 0, 0);
    let mut f = 0u32;
    unsafe {
        if ext {
            glGenFramebuffersEXT(1, &mut f);
            glBindFramebufferEXT(GL_FRAMEBUFFER, f);
        } else {
            glGenFramebuffers(1, &mut f);
            glBindFramebuffer(GL_FRAMEBUFFER, f);
        }
    }
    vngl_make_current(0, 0, 0, 0);
    f
}

/// Whether `ctx` sees framebuffer `f` (glIsFramebuffer, glIsFramebufferEXT).
fn sees_fbo(ctx: u32, f: u32) -> (bool, bool) {
    vngl_make_current(ctx, 0, 0, 0);
    let r = (glIsFramebuffer(f) == 1, glIsFramebufferEXT(f) == 1);
    vngl_make_current(0, 0, 0, 0);
    r
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
        // framebuffers: (name, made on context, EXT's (seen by the group) or the context's own)
        let mut fbos: Vec<(u32, u32, bool)> = Vec::new();
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
                    let empty = !made.iter().any(|m| m.1 == gb) && !fbos.iter().any(|f| f.2 && ctxs.iter().any(|c| c.0 == f.1 && c.1 == gb));
                    let want = ga == gb || (alone && empty);
                    prop_assert_eq!(vngl_share_lists(ctxs[a].0, ctxs[b].0) == 1, want, "{:?}", o);
                    if want {
                        // (what the context's EXT framebuffers were: none, it had no objects)
                        ctxs[b].1 = ga;
                    }
                }
                Op::Fbo(i, ext) => {
                    let Some(k) = pick(&ctxs, i) else { continue };
                    fbos.push((fbo_on(ctxs[k].0, ext), ctxs[k].0, ext));
                }
                Op::Delete(i) => {
                    let Some(k) = pick(&ctxs, i) else { continue };
                    let (id, group) = ctxs.remove(k);
                    vngl_context_destroy(id);
                    fbos.retain(|f| f.2 || f.1 != id);
                    if !ctxs.iter().any(|c| c.1 == group) {
                        made.retain(|m| m.1 != group);
                        fbos.retain(|f| !f.2 || f.1 != id);
                    } else {
                        // (EXT framebuffers stay the group's: another context of it holds them)
                        for f in fbos.iter_mut().filter(|f| f.2 && f.1 == id) {
                            f.1 = ctxs.iter().find(|c| c.1 == group).map_or(0, |c| c.0);
                        }
                    }
                }
            }
            let group_of = |ctx: u32| ctxs.iter().find(|c| c.0 == ctx).map(|c| c.1);
            for &(ctx, group) in &ctxs {
                for &(f, _, _) in &fbos {
                    let core = fbos.iter().any(|x| !x.2 && x.1 == ctx && x.0 == f);
                    let ext = fbos.iter().any(|x| x.2 && group_of(x.1) == Some(group) && x.0 == f);
                    prop_assert_eq!(sees_fbo(ctx, f), (core, ext), "context {} framebuffer {} after {:?}", ctx, f, o);
                }
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
