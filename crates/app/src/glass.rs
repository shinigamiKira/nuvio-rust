//! The blur behind glass panels. egui draws everything in one pass, so there is no ready-made "what is behind me": a
//! paint callback copies the pixels under the panel out of the framebuffer, blurs them at a quarter of the size with a
//! few cheap gaussian passes, and draws the result back through a rounded-rectangle mask. The tint, border and shadow
//! of the panel are ordinary egui shapes painted over it.

use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui::{PaintCallback, PaintCallbackInfo, Rect, Ui};
use eframe::egui_glow::{self, glow};
use glow::HasContext;

const VERTEX: &str = r#"#version 330 core
out vec2 uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}"#;

const BLUR: &str = r#"#version 330 core
in vec2 uv;
out vec4 color;
uniform sampler2D tex;
uniform vec2 dir;
void main() {
    vec4 c = texture(tex, uv) * 0.2270270270;
    c += (texture(tex, uv + dir * 1.3846153846) + texture(tex, uv - dir * 1.3846153846)) * 0.3162162162;
    c += (texture(tex, uv + dir * 3.2307692308) + texture(tex, uv - dir * 3.2307692308)) * 0.0702702703;
    color = c;
}"#;

const COMPOSITE: &str = r#"#version 330 core
in vec2 uv;
out vec4 color;
uniform sampler2D tex;
uniform vec2 size_px;
uniform float radius_px;
uniform float strength;
void main() {
    vec2 p = uv * size_px;
    vec2 half_size = size_px * 0.5;
    float distance = length(max(abs(p - half_size) - (half_size - vec2(radius_px)), 0.0)) - radius_px;
    float coverage = clamp(0.5 - distance, 0.0, 1.0) * strength;
    vec3 rgb = texture(tex, uv).rgb;
    float grey = dot(rgb, vec3(0.299, 0.587, 0.114));
    // Frosted glass lifts the colour of what is behind it a little.
    rgb = mix(vec3(grey), rgb, 1.35) * 0.92;
    color = vec4(rgb * coverage, coverage);
}"#;

struct Target {
    texture: glow::Texture,
    framebuffer: Option<glow::Framebuffer>,
    width: i32,
    height: i32,
}

impl Target {
    unsafe fn new(gl: &glow::Context, framebuffer: bool) -> Result<Self, String> {
        // SAFETY: plain object creation on the current context.
        unsafe {
            Ok(Self {
                texture: gl.create_texture()?,
                framebuffer: if framebuffer { Some(gl.create_framebuffer()?) } else { None },
                width: 0,
                height: 0,
            })
        }
    }

    unsafe fn resize(&mut self, gl: &glow::Context, width: i32, height: i32) {
        if self.width == width && self.height == height {
            return;
        }
        // SAFETY: the texture and framebuffer belong to this context.
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, width, height, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
            for (parameter, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, value as i32);
            }
            if let Some(framebuffer) = self.framebuffer {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(self.texture), 0);
            }
        }
        self.width = width;
        self.height = height;
    }
}

struct Program {
    program: glow::Program,
    tex: Option<glow::UniformLocation>,
    dir: Option<glow::UniformLocation>,
    size_px: Option<glow::UniformLocation>,
    radius_px: Option<glow::UniformLocation>,
    strength: Option<glow::UniformLocation>,
}

impl Program {
    unsafe fn new(gl: &glow::Context, fragment: &str) -> Result<Self, String> {
        // SAFETY: shader objects are created, compiled and deleted on the current context.
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, source) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, fragment)] {
                let shader = gl.create_shader(kind)?;
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    return Err(gl.get_shader_info_log(shader));
                }
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }
            Ok(Self {
                program,
                tex: gl.get_uniform_location(program, "tex"),
                dir: gl.get_uniform_location(program, "dir"),
                size_px: gl.get_uniform_location(program, "size_px"),
                radius_px: gl.get_uniform_location(program, "radius_px"),
                strength: gl.get_uniform_location(program, "strength"),
            })
        }
    }
}

struct Gl {
    blur: Program,
    composite: Program,
    vao: glow::VertexArray,
    capture: Target,
    half: Target,
    quarter_a: Target,
    quarter_b: Target,
}

impl Gl {
    unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        // SAFETY: object creation on the current context.
        unsafe {
            Ok(Self {
                blur: Program::new(gl, BLUR)?,
                composite: Program::new(gl, COMPOSITE)?,
                vao: gl.create_vertex_array()?,
                capture: Target::new(gl, false)?,
                half: Target::new(gl, true)?,
                quarter_a: Target::new(gl, true)?,
                quarter_b: Target::new(gl, true)?,
            })
        }
    }

    /// One full-screen pass of `program` from `source` into `target`.
    unsafe fn pass(&self, gl: &glow::Context, program: &Program, source: glow::Texture, target: &Target, direction: (f32, f32)) {
        // SAFETY: both textures and the framebuffer belong to this context and are complete.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, target.framebuffer);
            gl.viewport(0, 0, target.width, target.height);
            gl.use_program(Some(program.program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(source));
            gl.uniform_1_i32(program.tex.as_ref(), 0);
            gl.uniform_2_f32(program.dir.as_ref(), direction.0, direction.1);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    unsafe fn paint(&mut self, gl: &glow::Context, info: &PaintCallbackInfo, radius_px: f32, strength: f32) {
        let viewport = info.viewport_in_pixels();
        let (width, height) = (viewport.width_px, viewport.height_px);
        if width < 16 || height < 16 {
            return;
        }
        // SAFETY: every object used below was created on this context by `Gl::new`.
        unsafe {
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.vao));

            // 1. Copy the pixels under the panel.
            self.capture.resize(gl, width, height);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.capture.texture));
            gl.copy_tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, viewport.left_px, viewport.from_bottom_px, width, height);

            // 2. Shrink to a half, then a quarter: the bilinear taps average four texels each time.
            let (half_w, half_h) = ((width / 2).max(2), (height / 2).max(2));
            let (quarter_w, quarter_h) = ((width / 4).max(2), (height / 4).max(2));
            self.half.resize(gl, half_w, half_h);
            self.quarter_a.resize(gl, quarter_w, quarter_h);
            self.quarter_b.resize(gl, quarter_w, quarter_h);
            self.pass(gl, &self.blur, self.capture.texture, &self.half, (0.0, 0.0));
            self.pass(gl, &self.blur, self.half.texture, &self.quarter_a, (0.0, 0.0));

            // 3. Gaussian, twice over: horizontal then vertical each round.
            let (step_x, step_y) = (1.0 / quarter_w as f32, 1.0 / quarter_h as f32);
            for _ in 0..2 {
                self.pass(gl, &self.blur, self.quarter_a.texture, &self.quarter_b, (step_x, 0.0));
                self.pass(gl, &self.blur, self.quarter_b.texture, &self.quarter_a, (0.0, step_y));
            }

            // 4. Back to the window, through the rounded mask.
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(viewport.left_px, viewport.from_bottom_px, width, height);
            let clip = info.clip_rect_in_pixels();
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(clip.left_px, clip.from_bottom_px, clip.width_px, clip.height_px);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.use_program(Some(self.composite.program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.quarter_a.texture));
            gl.uniform_1_i32(self.composite.tex.as_ref(), 0);
            gl.uniform_2_f32(self.composite.size_px.as_ref(), width as f32, height as f32);
            gl.uniform_1_f32(self.composite.radius_px.as_ref(), radius_px);
            gl.uniform_1_f32(self.composite.strength.as_ref(), strength);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
        }
    }
}

enum State {
    Unstarted,
    Ready(Box<Gl>),
    Unavailable,
}

static GLASS: OnceLock<Arc<Mutex<State>>> = OnceLock::new();

fn shared() -> Arc<Mutex<State>> {
    GLASS.get_or_init(|| Arc::new(Mutex::new(State::Unstarted))).clone()
}

/// Paints the blurred backdrop for a panel at `rect`. Call it before drawing the panel's own fill, so the fill sits on top.
pub fn backdrop(ui: &Ui, rect: Rect, corner_points: f32, strength: f32) {
    let shared = shared();
    let radius_px = corner_points * ui.ctx().pixels_per_point();
    let callback = egui_glow::CallbackFn::new(move |info, painter| {
        let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*state, State::Unstarted) {
            // SAFETY: a paint callback runs with the window's GL context current.
            *state = match unsafe { Gl::new(painter.gl()) } {
                Ok(gl) => State::Ready(Box::new(gl)),
                Err(error) => {
                    tracing::warn!(%error, "glass blur is not available, panels stay translucent");
                    State::Unavailable
                }
            };
        }
        if let State::Ready(gl) = &mut *state {
            // SAFETY: as above.
            unsafe { gl.paint(painter.gl(), &info, radius_px, strength) };
        }
    });
    ui.painter().add(PaintCallback { rect, callback: Arc::new(callback) });
}
