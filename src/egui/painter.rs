use std::collections::HashMap;

use egui::epaint::Vertex;
use miniquad::{
    Backend, Bindings, BlendFactor, BlendState, BlendValue, BufferLayout, BufferSource, BufferType,
    BufferUsage, Equation, Pipeline, PipelineParams, RenderingBackend, ShaderSource, TextureId,
    UniformsSource, VertexAttribute, VertexFormat,
};

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
        let shader = ctx
            .new_shader(
                source,
                miniquad::ShaderMeta {
                    images: vec!["u_sampler".to_owned()],
                    uniforms: miniquad::UniformBlockLayout {
                        uniforms: vec![miniquad::UniformDesc::new(
                            "u_screen_size",
                            miniquad::UniformType::Float2,
                        )],
                    },
                },
            )
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

        let filter = match delta.options.magnification {
            egui::TextureFilter::Nearest => miniquad::FilterMode::Nearest,
            egui::TextureFilter::Linear => miniquad::FilterMode::Linear,
        };
        let texture = ctx.new_texture_from_data_and_format(
            data,
            miniquad::TextureParams {
                format: miniquad::TextureFormat::RGBA8,
                wrap: miniquad::TextureWrap::Clamp,
                min_filter: filter,
                mag_filter: filter,
                width: width as _,
                height: height as _,
                ..Default::default()
            },
        );

        if let Some(previous) = self.textures.insert(id, texture) {
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
        let screen_size_points = (
            screen_size.0 / egui_ctx.pixels_per_point(),
            screen_size.1 / egui_ctx.pixels_per_point(),
        );

        ctx.begin_default_pass(miniquad::PassAction::Nothing);
        ctx.apply_pipeline(&self.pipeline);
        ctx.apply_uniforms(UniformsSource::table(&shader::Uniforms {
            u_screen_size: screen_size_points,
        }));

        for egui::ClippedPrimitive {
            clip_rect,
            primitive,
        } in primitives
        {
            match primitive {
                egui::epaint::Primitive::Mesh(mesh) => {
                    self.paint_mesh(
                        ctx,
                        screen_size,
                        egui_ctx.pixels_per_point(),
                        clip_rect,
                        mesh,
                    );
                }
                egui::epaint::Primitive::Callback(_) => {
                    tracing::warn!("egui paint callbacks are not supported by the local painter");
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

            let egui::TextureId::Managed(_) = mesh.texture_id else {
                tracing::warn!("egui user textures are not supported by the local painter");
                continue;
            };
            let Some(texture) = self.textures.get(&mesh.texture_id).copied() else {
                tracing::warn!("egui texture {:?} was not found", mesh.texture_id);
                continue;
            };
            self.bindings.images[0] = texture;

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
    uniform sampler2D u_sampler;
    precision highp float;

    varying vec2 v_tc;
    varying vec4 v_rgba_in_gamma;

    void main() {
        vec4 texture_in_gamma = texture2D(u_sampler, v_tc);
        gl_FragColor = v_rgba_in_gamma * texture_in_gamma;
    }
    "#;

    #[repr(C)]
    pub(super) struct Uniforms {
        pub(super) u_screen_size: (f32, f32),
    }
}
