// use bevy::diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin};
use bevy::prelude::{
    default, App, Color, Commands, DespawnRecursiveExt, Entity, Input, KeyCode, PreUpdate, Query,
    Res, ResMut, Resource, SpatialBundle, Startup, Transform, Update, Vec2, With,
};
use bevy::text::{Text, Text2dBundle, TextSection, TextStyle};
use bevy::DefaultPlugins;
use bevy_cosmic_edit::*;

use bevy_prototype_lyon::prelude::{Fill, GeometryBuilder, PathBuilder, ShapeBundle, ShapePlugin};
use bevy_prototype_lyon::shapes::{Rectangle, RectangleOrigin};
use bevy_tokio_tasks::TokioTasksRuntime;
use brotli::Decompressor;
use std::io::Read;
use wasmtime::component::*;
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::preview2::command::sync;
use wasmtime_wasi::preview2::{Table, WasiCtx, WasiCtxBuilder, WasiView};
use wtransport::ClientConfig;
use wtransport::Endpoint;

use levo::portal::my_imports::Host;

#[path = "ui.rs"]
mod ui;
pub use ui::*;

bindgen!({
    world: "my-world",
    path: "../spec",
    async: false,
});

#[derive(Debug)]
struct FillRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(PartialEq, Debug)]
struct Arc {
    x: f32,
    y: f32,
    radius: f32,
    sweep_angle: f32,
    x_rotation: f32,
}

#[derive(PartialEq, Debug)]
struct CubicBezierTo {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
}

#[derive(PartialEq, Debug)]
struct Label {
    text: String,
    x: f32,
    y: f32,
    size: f32,
    color: String,
}

#[derive(Debug)]
enum HostEvent {
    Label(Label),
    FillStyle(String),
    FillRect(FillRect),
    MoveTo((f32, f32)),
    CubicBezierTo(CubicBezierTo),
    BeginPath,
    Arc(Arc),
    ClosePath,
    Fill,
}

struct MyCtx {
    table: Table,
    wasi: WasiCtx,
    queue: Vec<HostEvent>,
}

impl WasiView for MyCtx {
    fn table(&self) -> &Table {
        &self.table
    }
    fn table_mut(&mut self) -> &mut Table {
        &mut self.table
    }
    fn ctx(&self) -> &WasiCtx {
        &self.wasi
    }
    fn ctx_mut(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

// #[async_trait::async_trait]
impl Host for MyCtx {
    fn print(&mut self, from_wasm: String) -> wasmtime::Result<()> {
        dbg!(from_wasm);
        Ok(())
    }
    fn fill_style(&mut self, color: String) -> wasmtime::Result<()> {
        self.queue.push(HostEvent::FillStyle(color));
        Ok(())
    }
    fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32) -> wasmtime::Result<()> {
        self.queue.push(HostEvent::FillRect(FillRect {
            x,
            y,
            width,
            height,
        }));
        Ok(())
    }
    fn begin_path(&mut self) -> wasmtime::Result<()> {
        self.queue.push(HostEvent::BeginPath);
        Ok(())
    }
    fn arc(
        &mut self,
        x: f32,
        y: f32,
        radius: f32,
        sweep_angle: f32,
        x_rotation: f32,
    ) -> wasmtime::Result<()> {
        self.queue.push(HostEvent::Arc(Arc {
            x,
            y,
            radius,
            sweep_angle,
            x_rotation,
        }));
        Ok(())
    }
    fn close_path(&mut self) -> wasmtime::Result<()> {
        self.queue.push(HostEvent::ClosePath);
        Ok(())
    }
    fn fill(&mut self) -> wasmtime::Result<()> {
        self.queue.push(HostEvent::Fill);
        Ok(())
    }
    fn move_to(&mut self, x: f32, y: f32) -> wasmtime::Result<()> {
        self.queue.push(HostEvent::MoveTo((x, y)));
        Ok(())
    }
    fn cubic_bezier_to(
        &mut self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        x3: f32,
        y3: f32,
    ) -> wasmtime::Result<()> {
        self.queue.push(HostEvent::CubicBezierTo(CubicBezierTo {
            x1,
            y1,
            x2,
            y2,
            x3,
            y3,
        }));
        Ok(())
    }
    fn label(
        &mut self,
        text: String,
        x: f32,
        y: f32,
        size: f32,
        color: String,
    ) -> wasmtime::Result<()> {
        self.queue.push(HostEvent::Label(Label {
            text,
            x,
            y,
            size,
            color,
        }));
        Ok(())
    }
}

#[derive(Resource)]
struct WasmStore {
    store: Store<MyCtx>,
}

#[derive(Resource)]
struct WasmBindings {
    bindings: MyWorld,
    first_run: bool,
}

fn main() {
    App::new()
        // .add_plugins(FrameTimeDiagnosticsPlugin::default())
        // .add_plugins(LogDiagnosticsPlugin::default())
        .add_plugins(DefaultPlugins)
        .add_plugins(CosmicEditPlugin::default())
        .add_plugins(ShapePlugin)
        .add_systems(PreUpdate, clear)
        .add_systems(Update, handle_enter)
        .add_systems(Update, run_wasm_setup)
        .add_systems(Update, run_wasm_update)
        .add_systems(Update, handle_guest_event)
        .add_systems(Startup, setup)
        .add_plugins(bevy_tokio_tasks::TokioTasksPlugin {
            make_runtime: Box::new(|| {
                let mut runtime = tokio::runtime::Builder::new_multi_thread();
                runtime.enable_all();
                runtime.build().unwrap()
            }),
            ..bevy_tokio_tasks::TokioTasksPlugin::default()
        })
        .run();
}

#[derive(PartialEq)]
enum PathCommand {
    MoveTo((f32, f32)),
    CubicBezierTo(CubicBezierTo),
    Arc(Arc),
    Begin,
    Close,
}

#[derive(bevy::prelude::Component)]
struct GuestEntity;

fn clear(mut commands: Commands, guest_entites: Query<Entity, With<GuestEntity>>) {
    for entity in guest_entites.iter() {
        commands.entity(entity).despawn_recursive();
    }
}

fn handle_guest_event(mut commands: Commands, wasm_store: Option<ResMut<WasmStore>>) {
    let Some(mut wasm_store) = wasm_store else {
        return;
    };
    let queue = &mut wasm_store.store.data_mut().queue;
    let mut current_fill = None;
    let mut current_path = Vec::new();
    let mut w = -1.;
    let mut h = -1.;
    for r in queue.drain(..) {
        match r {
            HostEvent::FillStyle(c_str) => {
                let c_val = string_to_bevy_color(c_str);
                current_fill = Some(Fill::color(c_val))
            }
            HostEvent::FillRect(FillRect {
                x,
                y,
                width,
                height,
            }) => {
                w = width;
                h = height;
                let rect = Rectangle {
                    extents: Vec2::new(x + width, y + height),
                    origin: RectangleOrigin::Center,
                };
                commands.spawn((
                    ShapeBundle {
                        path: GeometryBuilder::build_as(&rect),
                        ..default()
                    },
                    current_fill.unwrap_or(Fill::color(Color::RED)),
                    GuestEntity,
                ));
                current_fill = None;
            }
            HostEvent::BeginPath => {
                current_path.push(PathCommand::Begin);
            }
            HostEvent::Arc(arc) => {
                current_path.push(PathCommand::Arc(arc));
            }
            HostEvent::ClosePath => {
                current_path.push(PathCommand::Close);
            }
            HostEvent::Fill => {
                let first = current_path.get(0);

                if let Some(PathCommand::Begin) = first {
                    let mut path_builder = PathBuilder::new();
                    for command in current_path.drain(..).skip(1) {
                        match command {
                            PathCommand::Arc(Arc {
                                x,
                                y,
                                radius,
                                sweep_angle,
                                x_rotation,
                            }) => {
                                path_builder.move_to(Vec2::new(x - w / 2., y + h / 2.));
                                path_builder.arc(
                                    Vec2::new(x + radius - w / 2., y + radius + h / 2.),
                                    Vec2::new(radius, radius),
                                    sweep_angle,
                                    x_rotation,
                                );
                            }
                            PathCommand::Begin => {
                                dbg!("path already created");
                            }
                            PathCommand::Close => {
                                path_builder.close();
                            }
                            PathCommand::MoveTo((x, y)) => {
                                path_builder.move_to(Vec2::new(x, y));
                            }
                            PathCommand::CubicBezierTo(CubicBezierTo {
                                x1,
                                y1,
                                x2,
                                y2,
                                x3,
                                y3,
                            }) => {
                                path_builder.cubic_bezier_to(
                                    Vec2::new(x1, y1),
                                    Vec2::new(x2, y2),
                                    Vec2::new(x3, y3),
                                );
                            }
                        }
                    }
                    let path = path_builder.build();
                    commands.spawn((
                        ShapeBundle {
                            spatial: SpatialBundle {
                                transform: Transform::from_xyz(0., 0., 0.001),
                                ..default()
                            },
                            path,
                            ..default()
                        },
                        current_fill.unwrap_or(Fill::color(Color::RED)),
                        GuestEntity,
                    ));
                    current_fill = None;
                } else {
                    eprintln!("path should start from begin");
                }
            }
            HostEvent::MoveTo((x, y)) => {
                current_path.push(PathCommand::MoveTo((x, y)));
            }
            HostEvent::CubicBezierTo(cbt) => {
                current_path.push(PathCommand::CubicBezierTo(cbt));
            }
            HostEvent::Label(Label {
                text,
                x,
                y,
                size,
                color,
            }) => {
                commands.spawn((
                    Text2dBundle {
                        text: Text {
                            sections: vec![TextSection::new(
                                text,
                                TextStyle {
                                    font_size: size,
                                    color: string_to_bevy_color(color),
                                    ..default()
                                },
                            )],
                            ..default()
                        },
                        transform: Transform::from_xyz(x, y, 0.01),
                        ..default()
                    },
                    GuestEntity,
                ));
            }
        }
    }
}

fn handle_enter(
    editor_q: Query<&CosmicEditor>,
    keys: Res<Input<KeyCode>>,
    runtime: ResMut<TokioTasksRuntime>,
) {
    if !keys.just_pressed(KeyCode::Return) {
        return;
    }
    for editor in editor_q.iter() {
        let text = editor.get_text();
        runtime.spawn_background_task(|ctx| async move {
            match get_wasm(ctx, text.clone()).await {
                Ok(_) => {}
                Err(e) => eprintln!("failed to get wasm for '{text}': {e}"),
            }
        });
    }
}

fn run_wasm_update(
    wasm_instance: Option<ResMut<WasmBindings>>,
    wasm_store: Option<ResMut<WasmStore>>,
) {
    if let Some(wasm_resource) = wasm_instance {
        let mut store = wasm_store.unwrap();
        let _ = wasm_resource.bindings.call_update(&mut store.store);
    }
}

fn run_wasm_setup(
    wasm_instance: Option<ResMut<WasmBindings>>,
    wasm_store: Option<ResMut<WasmStore>>,
) {
    if let Some(mut wasm_resource) = wasm_instance {
        if wasm_resource.first_run {
            wasm_resource.first_run = false;
            let mut store = wasm_store.unwrap();
            let _ = wasm_resource.bindings.call_setup(&mut store.store);
        }
    }
}

async fn get_wasm(
    mut ctx: bevy_tokio_tasks::TaskContext,
    host: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = ClientConfig::builder()
        .with_bind_default()
        .with_no_cert_validation() // FIXME: don't do it on prod!
        .enable_key_log() // TODO: this is just for debugging
        .build();

    let connection = Endpoint::client(config)
        .unwrap()
        .connect(format!("https://{}:4433", host))
        .await
        .unwrap();

    let mut stream = connection.open_bi().await.unwrap().await?;
    stream.0.write_all(b"WASM").await?;

    let initial_buffer_size = 65536;
    let mut buffer = Vec::with_capacity(initial_buffer_size);
    loop {
        let mut chunk = vec![0; 65536];
        match stream.1.read(&mut chunk).await? {
            Some(bytes_read) => {
                buffer.extend_from_slice(&chunk[..bytes_read]);
            }
            None => break, // End of stream
        }
    }

    // Decompress the received buffer using rust-brotli
    let mut decompressed_reader = Decompressor::new(buffer.as_slice(), 4096);
    let mut decoded_input = Vec::new();
    decompressed_reader.read_to_end(&mut decoded_input)?;

    // Set up Wasmtime components
    let mut config = Config::new();
    config.wasm_component_model(true).async_support(false);
    let engine = Engine::new(&config)?;
    let component = Component::new(&engine, decoded_input)?;

    // Set up Wasmtime linker
    let mut linker = Linker::new(&engine);
    sync::add_to_linker(&mut linker)?;
    let table = Table::new();
    let wasi = WasiCtxBuilder::new().build();
    MyWorld::add_to_linker(&mut linker, |state: &mut MyCtx| state)?;
    // Set up Wasmtime store
    let mut store = Store::new(
        &engine,
        MyCtx {
            table,
            wasi,
            queue: Vec::new(),
        },
    );
    let (bindings, _) = MyWorld::instantiate(&mut store, &component, &linker)?;

    ctx.run_on_main_thread(move |ctx| {
        if let Some(mut wasm_resource) = ctx.world.get_resource_mut::<WasmBindings>() {
            wasm_resource.bindings = bindings;
            wasm_resource.first_run = true;
        } else {
            ctx.world.insert_resource(WasmBindings {
                bindings,
                first_run: true,
            })
        }
        if let Some(mut wasm_resource) = ctx.world.get_resource_mut::<WasmStore>() {
            wasm_resource.store = store;
        } else {
            ctx.world.insert_resource(WasmStore { store })
        }
    })
    .await;

    Ok(())
}
