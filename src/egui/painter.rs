use std::collections::HashMap;

use egui::epaint::Vertex;
use miniquad::*;

pub(super) struct Painter {
    pipeline: Pipeline,
    bindings: Bindings,
    textures: HashMap<egui::TextureId, TextureId>,
}

impl Painter {
    pub(super) fn new(ctx: &mut dyn RenderingBackend) -> Self {
        let source = match ctx.info().backend {
            Backend::OpenGl => ShaderSource::Glsl {
                vertex: shader::VERTEX,
                fragment: shader::FRAGMENT,
            },
            Backend::Metal => unimplemented!("egui painter currently supports OpenGL only"),
        };
        let shader_meta = ShaderMeta {
            images: vec!["u_sampler".to_owned()],
            uniforms: UniformBlockLayout {
                uniforms: vec![
                    UniformDesc::new("u_screen_size", UniformType::Float2),
                    UniformDesc::new("u_texture", UniformType::Float4),
                    UniformDesc::new("u_sample_mode", UniformType::Float1),
                ],
            },
        };
        let shader = ctx
            .new_shader(source, shader_meta)
            .expect("create egui shader");
        let pipeline = ctx.new_pipeline(
            &[BufferLayout::default()],
            &[
                VertexAttribute::new("a_pos", VertexFormat::Float2),
                VertexAttribute::new("a_tc", VertexFormat::Float2),
                VertexAttribute::new("a_srgba", VertexFormat::Byte4),
            ],
            shader,
            PipelineParams {
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::One,
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                cull_face: miniquad::CullFace::Nothing,
                ..Default::default()
            },
        );

        Self {
            pipeline,
            bindings: Bindings {
                vertex_buffers: vec![ctx.new_buffer(
                    BufferType::VertexBuffer,
                    BufferUsage::Stream,
                    BufferSource::empty::<Vertex>(32 * 1024),
                )],
                index_buffer: ctx.new_buffer(
                    BufferType::IndexBuffer,
                    BufferUsage::Stream,
                    BufferSource::empty::<u16>(32 * 1024),
                ),
                images: vec![ctx.new_texture_from_rgba8(1, 1, &[255, 255, 255, 255])],
            },
            textures: HashMap::new(),
        }
    }

    fn set_texture(
        &mut self,
        ctx: &mut dyn RenderingBackend,
        id: egui::TextureId,
        delta: &egui::epaint::ImageDelta,
    ) {
        let [width, height] = delta.image.size();
        let data = match &delta.image {
            egui::ImageData::Color(image) => {
                debug_assert_eq!(image.width() * image.height(), image.pixels.len());
                bytemuck::cast_slice(image.pixels.as_ref())
            }
        };

        if let Some([x, y]) = delta.pos {
            if let Some(texture) = self.textures.get(&id) {
                ctx.texture_update_part(*texture, x as _, y as _, width as _, height as _, data);
            } else {
                tracing::warn!("egui texture {id:?} was not found for partial update");
            }
            return;
        }

        let wrap = match delta.options.wrap_mode {
            egui::TextureWrapMode::ClampToEdge => TextureWrap::Clamp,
            egui::TextureWrapMode::Repeat => TextureWrap::Repeat,
            egui::TextureWrapMode::MirroredRepeat => TextureWrap::Mirror,
        };

        let params = TextureParams {
            format: TextureFormat::RGBA8,
            wrap,
            min_filter: match delta.options.minification {
                egui::TextureFilter::Nearest => FilterMode::Nearest,
                egui::TextureFilter::Linear => FilterMode::Linear,
            },
            mag_filter: match delta.options.magnification {
                egui::TextureFilter::Nearest => FilterMode::Nearest,
                egui::TextureFilter::Linear => FilterMode::Linear,
            },
            width: width as _,
            height: height as _,
            ..Default::default()
        };
        let texture_id = ctx.new_texture_from_data_and_format(data, params);

        if let Some(previous) = self.textures.insert(id, texture_id) {
            ctx.delete_texture(previous);
        }
    }

    pub(super) fn paint(
        &mut self,
        ctx: &mut dyn RenderingBackend,
        primitives: Vec<egui::ClippedPrimitive>,
        textures_delta: &egui::TexturesDelta,
        egui_ctx: &egui::Context,
    ) {
        for (id, delta) in &textures_delta.set {
            self.set_texture(ctx, *id, delta);
        }

        let screen_size = miniquad::window::screen_size();
        let pixels_per_point = egui_ctx.pixels_per_point();
        ctx.begin_default_pass(miniquad::PassAction::Nothing);
        ctx.apply_pipeline(&self.pipeline);

        for egui::ClippedPrimitive {
            clip_rect,
            primitive,
        } in primitives
        {
            match primitive {
                egui::epaint::Primitive::Mesh(mesh) => {
                    self.paint_mesh(ctx, screen_size, pixels_per_point, clip_rect, mesh);
                }
                egui::epaint::Primitive::Callback(_) => {
                    tracing::warn!("egui paint callbacks are not supported");
                }
            }
        }

        ctx.end_render_pass();

        for &id in &textures_delta.free {
            if let Some(texture) = self.textures.remove(&id) {
                ctx.delete_texture(texture);
            }
        }
    }

    fn paint_mesh(
        &mut self,
        ctx: &mut dyn RenderingBackend,
        screen_size: (f32, f32),
        pixels_per_point: f32,
        clip_rect: egui::Rect,
        mesh: egui::epaint::Mesh,
    ) {
        debug_assert!(mesh.is_valid());

        let egui::TextureId::Managed(_) = mesh.texture_id else {
            tracing::warn!("egui user textures are not supported");
            return;
        };
        let Some(texture) = self.textures.get(&mesh.texture_id).copied() else {
            tracing::warn!("egui texture {:?} was not found", mesh.texture_id);
            return;
        };
        self.bindings.images[0] = texture;

        let mut position_rect = egui::Rect::NOTHING;
        let mut uv_rect = egui::Rect::NOTHING;
        for vertex in &mesh.vertices {
            position_rect.extend_with(vertex.pos);
            uv_rect.extend_with(vertex.uv);
        }

        let texture_params = ctx.texture_params(texture);
        let texture_size = egui::vec2(texture_params.width as f32, texture_params.height as f32);
        let screen_size_pixels = position_rect.size() * pixels_per_point;
        let source_size = uv_rect.size() * texture_size;
        let mut scale = egui::Vec2::ONE;
        if source_size.x > f32::EPSILON {
            scale.x = screen_size_pixels.x / source_size.x;
        }
        if source_size.y > f32::EPSILON {
            scale.y = screen_size_pixels.y / source_size.y;
        }

        let is_downscaled = scale.x < 1.0 || scale.y < 1.0;
        let is_integer_scale =
            scale.x >= 1.0 && scale.y >= 1.0 && (scale - scale.round()).abs().max_elem() <= 0.001;
        let sample_mode = if mesh.texture_id == egui::TextureId::default() {
            1.0
        } else if is_downscaled {
            match texture_params.min_filter {
                FilterMode::Nearest => 0.0,
                FilterMode::Linear => 3.0,
            }
        } else if is_integer_scale {
            0.0
        } else {
            match texture_params.mag_filter {
                FilterMode::Nearest => 0.0,
                FilterMode::Linear => 2.0,
            }
        };
        ctx.apply_uniforms(UniformsSource::table(&shader::Uniforms {
            u_screen_size: (
                screen_size.0 / pixels_per_point,
                screen_size.1 / pixels_per_point,
            ),
            u_texture: (texture_size.x, texture_size.y, scale.x, scale.y),
            u_sample_mode: sample_mode,
        }));

        let min_x = (pixels_per_point * clip_rect.min.x).clamp(0.0, screen_size.0);
        let min_y = (pixels_per_point * clip_rect.min.y).clamp(0.0, screen_size.1);
        let max_x = (pixels_per_point * clip_rect.max.x).clamp(min_x, screen_size.0);
        let max_y = (pixels_per_point * clip_rect.max.y).clamp(min_y, screen_size.1);
        let min_x = min_x.round() as u32;
        let min_y = min_y.round() as u32;
        let max_x = max_x.round() as u32;
        let max_y = max_y.round() as u32;

        ctx.apply_scissor_rect(
            min_x as i32,
            (screen_size.1 as u32 - max_y) as i32,
            (max_x - min_x) as i32,
            (max_y - min_y) as i32,
        );

        for mesh in mesh.split_to_u16() {
            debug_assert!(mesh.is_valid());
            self.ensure_buffer_capacity(ctx, mesh.vertices.len(), mesh.indices.len());
            ctx.buffer_update(
                self.bindings.vertex_buffers[0],
                BufferSource::slice(&mesh.vertices),
            );
            ctx.buffer_update(
                self.bindings.index_buffer,
                BufferSource::slice(&mesh.indices),
            );
            ctx.apply_bindings(&self.bindings);
            ctx.draw(0, mesh.indices.len() as i32, 1);
        }
    }

    fn ensure_buffer_capacity(
        &mut self,
        ctx: &mut dyn RenderingBackend,
        vertex_count: usize,
        index_count: usize,
    ) {
        let vertex_bytes = vertex_count * std::mem::size_of::<Vertex>();
        if ctx.buffer_size(self.bindings.vertex_buffers[0]) < vertex_bytes {
            ctx.delete_buffer(self.bindings.vertex_buffers[0]);
            self.bindings.vertex_buffers[0] = ctx.new_buffer(
                BufferType::VertexBuffer,
                BufferUsage::Stream,
                BufferSource::empty::<Vertex>(vertex_count),
            );
        }

        let index_bytes = index_count * std::mem::size_of::<u16>();
        if ctx.buffer_size(self.bindings.index_buffer) < index_bytes {
            ctx.delete_buffer(self.bindings.index_buffer);
            self.bindings.index_buffer = ctx.new_buffer(
                BufferType::IndexBuffer,
                BufferUsage::Stream,
                BufferSource::empty::<u16>(index_count),
            );
        }
    }
}

mod shader {
    pub(super) const VERTEX: &str = r#"
    #version 100
    uniform vec2 u_screen_size;

    attribute vec2 a_pos;
    attribute vec2 a_tc;
    attribute vec4 a_srgba;

    varying vec2 v_tc;
    varying vec4 v_rgba_in_gamma;

    void main() {
        gl_Position = vec4(
            2.0 * a_pos.x / u_screen_size.x - 1.0,
            1.0 - 2.0 * a_pos.y / u_screen_size.y,
            0.0,
            1.0);
        v_rgba_in_gamma = a_srgba / 255.0;
        v_tc = a_tc;
    }
    "#;

    pub(super) const FRAGMENT: &str = r#"
    #version 100
    precision highp float;
    precision mediump int;

    uniform sampler2D u_sampler;
    uniform vec4 u_texture;
    uniform float u_sample_mode;

    varying vec2 v_tc;
    varying vec4 v_rgba_in_gamma;

    vec4 sample_area(vec2 uv) {
        vec2 texture_size = u_texture.xy;
        vec2 scale = max(u_texture.zw, vec2(1e-4));
        vec2 footprint = 1.0 / scale;
        footprint = max(vec2(1.0), footprint);
        vec2 sample_step = footprint / (4.0 * texture_size);

        vec4 color = vec4(0.0);
        for (int i = 0; i < 16; i++) {
            float index = float(i);
            vec2 grid = vec2(mod(index, 4.0), floor(index / 4.0));
            vec2 sample_uv = uv + (grid - 1.5) * sample_step;
            color += texture2D(u_sampler, sample_uv);
        }

        return color / 16.0;
    }

    vec4 sample_texture(vec2 uv) {
        if (u_sample_mode > 2.5) {
            return sample_area(uv);
        }

        vec2 texture_size = u_texture.xy;
        vec2 texel = uv * texture_size - 0.5;
        vec2 blend = fract(texel);

        if (u_sample_mode < 0.5) {
            blend = floor(blend + 0.5);
        } else if (u_sample_mode > 1.5) {
            vec2 sharpness = max(u_texture.zw, vec2(1.0));
            blend = (blend - 0.5) * sharpness + 0.5;
            blend = clamp(blend, 0.0, 1.0);
        }

        uv = (floor(texel) + blend + 0.5) / texture_size;
        return texture2D(u_sampler, uv);
    }

    void main() {
        gl_FragColor = v_rgba_in_gamma * sample_texture(v_tc);
    }
    "#;

    #[repr(C)]
    pub(super) struct Uniforms {
        pub(super) u_screen_size: (f32, f32),
        pub(super) u_texture: (f32, f32, f32, f32),
        pub(super) u_sample_mode: f32,
    }
}
