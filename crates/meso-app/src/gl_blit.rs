//! GL texture blit helper for the `GLArea` fast display path.
//!
//! `GlBlitContext` owns a `glow::Context`, a compiled GLSL ES 3.00 shader
//! program, a fullscreen-quad VAO/VBO, and an RGBA texture. Call `blit()` to
//! upload new Cairo-ARGB32 (BGRA in memory on LE) pixel data and draw it with
//! the inverse of the Cairo pan/zoom transform so the GL display matches what
//! `DrawingArea` would produce.
//!
//! Cairo ARgb32 stores pixels as BGRA in memory (little-endian). GLES 3.0 does
//! not support `GL_BGRA` as an upload format, so we upload as `GL_RGBA` and
//! swap the R/B channels in the fragment shader.
//!
//! Lifecycle: create from `GLArea::connect_realize`; drop from
//! `GLArea::connect_unrealize` to release GL resources while the context is
//! still active.

use anyhow::{anyhow, bail, Result};
use std::ffi::CString;
use std::os::raw::{c_char, c_void};

#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetProcAddress(name: *const c_char) -> *const c_void;
}

#[link(name = "GL")]
unsafe extern "C" {
    fn glXGetProcAddressARB(name: *const u8) -> *const c_void;
}

unsafe fn platform_loader(name: &str) -> *const c_void {
    let cstr = CString::new(name).expect("GL symbol contained interior NUL");
    let egl = unsafe { eglGetProcAddress(cstr.as_ptr()) };
    if !egl.is_null() {
        return egl;
    }
    unsafe { glXGetProcAddressARB(cstr.as_ptr() as *const u8) }
}

const VERT_SRC: &str = r#"#version 300 es
in vec2 a_pos;
void main() {
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

const FRAG_SRC: &str = r#"#version 300 es
precision mediump float;
out vec4 out_color;
uniform sampler2D u_texture;
uniform vec2  u_size;
uniform vec2  u_offset;
uniform float u_zoom;
uniform vec2  u_zoom_center;
void main() {
    vec2 screen = gl_FragCoord.xy;
    screen.y = u_size.y - screen.y;
    vec2 tex = (screen - u_offset - u_zoom_center) / u_zoom + u_zoom_center;
    vec2 uv = tex / u_size;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        out_color = vec4(0.0, 0.0, 0.0, 1.0);
    } else {
        // Cairo ARgb32 is BGRA in memory; uploaded as GL_RGBA so R/B are swapped.
        out_color = texture(u_texture, uv).bgra;
    }
}
"#;

pub struct GlBlitContext {
    gl: glow::Context,
    program: glow::NativeProgram,
    vao: glow::NativeVertexArray,
    #[allow(dead_code)]
    vbo: glow::NativeBuffer,
    texture: glow::NativeTexture,
    loc_texture: glow::NativeUniformLocation,
    loc_size: glow::NativeUniformLocation,
    loc_offset: glow::NativeUniformLocation,
    loc_zoom: glow::NativeUniformLocation,
    loc_zoom_center: glow::NativeUniformLocation,
}

impl GlBlitContext {
    /// Create a new `GlBlitContext`. **Must** be called after `gl_area.make_current()`.
    ///
    /// # Safety
    /// A valid OpenGL ES 3.0+ context must be current on the calling thread.
    pub unsafe fn new() -> Result<Self> {
        use glow::HasContext;

        let gl = unsafe { glow::Context::from_loader_function(|name| platform_loader(name)) };

        let vert = unsafe { compile_shader(&gl, glow::VERTEX_SHADER, VERT_SRC)? };
        let frag = unsafe { compile_shader(&gl, glow::FRAGMENT_SHADER, FRAG_SRC)? };
        let program = unsafe {
            gl.create_program()
                .map_err(|e| anyhow!("create program: {e}"))?
        };
        unsafe {
            gl.attach_shader(program, vert);
            gl.attach_shader(program, frag);
            gl.link_program(program);
            gl.delete_shader(vert);
            gl.delete_shader(frag);
        }
        if unsafe { !gl.get_program_link_status(program) } {
            bail!("shader link: {}", unsafe {
                gl.get_program_info_log(program)
            });
        }

        let verts: [f32; 12] = [
            -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0,
        ];
        let vao = unsafe { gl.create_vertex_array().map_err(|e| anyhow!("vao: {e}"))? };
        let vbo = unsafe { gl.create_buffer().map_err(|e| anyhow!("vbo: {e}"))? };
        unsafe {
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&verts),
                glow::STATIC_DRAW,
            );
        }
        let a_pos = unsafe {
            gl.get_attrib_location(program, "a_pos")
                .ok_or_else(|| anyhow!("a_pos not found"))?
        };
        unsafe {
            gl.enable_vertex_attrib_array(a_pos);
            gl.vertex_attrib_pointer_f32(a_pos, 2, glow::FLOAT, false, 8, 0);
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }

        let texture = unsafe { gl.create_texture().map_err(|e| anyhow!("tex: {e}"))? };
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }

        macro_rules! uloc {
            ($name:literal) => {
                unsafe {
                    gl.get_uniform_location(program, $name)
                        .ok_or_else(|| anyhow!(concat!("uniform ", $name, " not found")))?
                }
            };
        }
        let loc_texture = uloc!("u_texture");
        let loc_size = uloc!("u_size");
        let loc_offset = uloc!("u_offset");
        let loc_zoom = uloc!("u_zoom");
        let loc_zoom_center = uloc!("u_zoom_center");

        Ok(Self {
            gl,
            program,
            vao,
            vbo,
            texture,
            loc_texture,
            loc_size,
            loc_offset,
            loc_zoom,
            loc_zoom_center,
        })
    }

    /// Upload Cairo-ARGB32 (BGRA on LE) pixel data and draw with the
    /// pan/zoom transform.
    ///
    /// # Safety
    /// The correct GL context must be current on the calling thread.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn blit(
        &self,
        bgra: &[u8],
        img_w: u32,
        img_h: u32,
        vp_w: i32,
        vp_h: i32,
        drag_x: f64,
        drag_y: f64,
        zoom: f64,
        zoom_cx: f64,
        zoom_cy: f64,
    ) {
        use glow::HasContext;

        let gl = &self.gl;
        let w = img_w as i32;
        let h = img_h as i32;

        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w,
                h,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                Some(bgra),
            );
            gl.viewport(0, 0, vp_w, vp_h);
            gl.use_program(Some(self.program));
            gl.uniform_1_i32(Some(&self.loc_texture), 0);
            gl.uniform_2_f32(Some(&self.loc_size), vp_w as f32, vp_h as f32);
            gl.uniform_2_f32(Some(&self.loc_offset), drag_x as f32, drag_y as f32);
            gl.uniform_1_f32(Some(&self.loc_zoom), zoom as f32);
            gl.uniform_2_f32(Some(&self.loc_zoom_center), zoom_cx as f32, zoom_cy as f32);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }
}

impl Drop for GlBlitContext {
    fn drop(&mut self) {
        use glow::HasContext;

        // GL resources are cleaned up while the GLArea context is current.
        unsafe {
            self.gl.delete_texture(self.texture);
            self.gl.delete_buffer(self.vbo);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_program(self.program);
        }
    }
}

unsafe fn compile_shader(gl: &glow::Context, kind: u32, src: &str) -> Result<glow::NativeShader> {
    use glow::HasContext;

    let sh = unsafe {
        gl.create_shader(kind)
            .map_err(|e| anyhow!("create shader: {e}"))?
    };
    unsafe {
        gl.shader_source(sh, src);
        gl.compile_shader(sh);
    }
    if unsafe { !gl.get_shader_compile_status(sh) } {
        bail!("shader compile: {}", unsafe { gl.get_shader_info_log(sh) });
    }
    Ok(sh)
}
