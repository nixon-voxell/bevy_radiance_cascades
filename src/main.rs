use bevy::{
    // color::palettes::css,
    core_pipeline::{bloom::BloomSettings, smaa::SmaaSettings, tonemapping::Tonemapping},
    prelude::*,
    sprite::Mesh2dHandle,
    window::PrimaryWindow,
};
use bevy_motiongfx::{prelude::*, BevyMotionGfxPlugin};
// use rand::prelude::*;

mod debug_render_pipeline;
mod jfa;
mod mask2d;
mod math_util;
mod radiance_cascades;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(BevyMotionGfxPlugin)
        .insert_resource(Msaa::Off)
        .add_plugins(mask2d::Mask2dPrepassPlugin::<ColorMaterial>::default())
        .add_plugins(jfa::JfaPrepassPlugin)
        .add_plugins(radiance_cascades::RadianceCascadesPlugin)
        // .add_plugins(debug_render_pipeline::DebugRenderPipelinePlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, follow_mouse)
        .add_systems(Update, cascade_settings)
        .add_systems(Update, timeline_movement)
        .run();
}

#[derive(Component)]
pub struct Marked;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    // asset_server: Res<AssetServer>,
) {
    // camera
    commands.spawn((
        Camera2dBundle {
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, 5.0))
                .looking_at(Vec3::default(), Vec3::Y),
            camera: Camera {
                clear_color: Color::BLACK.into(),
                hdr: true,
                ..default()
            },
            tonemapping: Tonemapping::AcesFitted,
            ..default()
        },
        jfa::JfaPrepass,
        mask2d::Mask2dPrepass,
        radiance_cascades::RadianceCascadesConfig::default(),
        BloomSettings::default(),
        SmaaSettings::default(),
    ));

    const COUNT: usize = 4;
    const SPACING: f32 = 150.0;
    const OFFSET: Vec3 = Vec3::new(
        (COUNT as f32) * 0.5 * SPACING - SPACING * 0.5,
        (COUNT as f32) * 0.5 * SPACING - SPACING * 0.5,
        0.0,
    );

    for y in 0..COUNT {
        for x in 0..COUNT {
            commands.spawn((
                ColorMesh2dBundle {
                    mesh: Mesh2dHandle(meshes.add(Circle::new(15.0))),
                    material: materials.add(Color::linear_rgba(0.0, 0.0, 0.0, 1.0)),
                    transform: Transform::from_translation(
                        Vec3::new((x as f32) * SPACING, (y as f32) * SPACING, 0.0) - OFFSET,
                    ),
                    ..default()
                },
                mask2d::Mask2d,
            ));
        }
    }

    // let mesh = asset_server.load(GltfAssetLabel::Mesh(0).from_asset("Scene.glb"));
    // commands.spawn((
    //     ColorMesh2dBundle {
    //         mesh: Mesh2dHandle(mesh),
    //         material: materials.add(Color::linear_rgba(0.0, 0.0, 4.0, 1.0)),
    //         transform: Transform::from_xyz(0.0, 0.0, 0.1),
    //         ..default()
    //     },
    //     mask2d::Mask2d,
    // ));

    // Circle
    commands.spawn((
        ColorMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Circle::new(20.0))),
            material: materials.add(Color::linear_rgb(4.0, 2.0, 0.0)),
            transform: Transform::from_xyz(0.0, 0.0, 0.2),
            ..default()
        },
        mask2d::Mask2d,
        Marked,
    ));

    // Rect
    {
        const MAX: f32 = SPACING * COUNT as f32 * 0.55;
        let transform = Transform::from_xyz(-MAX, -MAX, 0.1);
        let material = ColorMaterial::from_color(Color::linear_rgb(0.0, 2.0, 3.0));

        let id = commands
            .spawn((
                ColorMesh2dBundle {
                    mesh: Mesh2dHandle(meshes.add(Rectangle {
                        half_size: Vec2::new(30.0, 30.0),
                    })),
                    material: materials.add(material.clone()),
                    transform,
                    ..default()
                },
                mask2d::Mask2d,
            ))
            .id();

        let mut rect = (id, (transform, material));
        let sequence = [
            [
                commands.play_motion(
                    rect.transform()
                        .to_translation_x(MAX)
                        .with_ease(ease::cubic::ease_in_out)
                        .animate(2.0),
                ),
                commands.play_motion(
                    act!(
                        (rect.id(), ColorMaterial),
                        start = { rect.get_mut::<ColorMaterial>() }.color,
                        end = Color::linear_rgb(0.0, 3.0, 2.0),
                    )
                    .animate(2.0),
                ),
            ]
            .all(),
            [
                commands.play_motion(
                    rect.transform()
                        .to_translation_y(MAX)
                        .with_ease(ease::cubic::ease_in_out)
                        .animate(2.0),
                ),
                commands.play_motion(
                    act!(
                        (rect.id(), ColorMaterial),
                        start = { rect.get_mut::<ColorMaterial>() }.color,
                        end = Color::linear_rgb(2.0, 3.0, 0.0),
                    )
                    .animate(2.0),
                ),
            ]
            .all(),
            [
                commands.play_motion(
                    rect.transform()
                        .to_translation_x(-MAX)
                        .with_ease(ease::cubic::ease_in_out)
                        .animate(2.0),
                ),
                commands.play_motion(
                    act!(
                        (rect.id(), ColorMaterial),
                        start = { rect.get_mut::<ColorMaterial>() }.color,
                        end = Color::linear_rgb(3.0, 2.0, 0.0),
                    )
                    .animate(2.0),
                ),
            ]
            .all(),
            [
                commands.play_motion(
                    rect.transform()
                        .to_translation_y(-MAX)
                        .with_ease(ease::cubic::ease_in_out)
                        .animate(2.0),
                ),
                commands.play_motion(
                    act!(
                        (rect.id(), ColorMaterial),
                        start = { rect.get_mut::<ColorMaterial>() }.color,
                        end = Color::linear_rgb(0.0, 2.0, 3.0),
                    )
                    .animate(2.0),
                ),
            ]
            .all(),
        ]
        .chain();

        commands.spawn(SequencePlayerBundle {
            sequence,
            ..default()
        });
    }

    // Sector
    {
        const MAX: f32 = SPACING * COUNT as f32 * 0.25;
        let transform = Transform::from_xyz(MAX, MAX, 0.1);
        let material = ColorMaterial::from_color(Color::linear_rgb(2.8, 1.2, 0.0));

        let id = commands
            .spawn((
                ColorMesh2dBundle {
                    mesh: Mesh2dHandle(
                        meshes.add(CircularSector::new(25.0, std::f32::consts::PI * 0.7)),
                    ),
                    material: materials.add(material.clone()),
                    transform,
                    ..default()
                },
                mask2d::Mask2d,
            ))
            .id();

        let mut sector = (id, (transform, material));

        let mut target = -MAX;
        let mut sequences = Vec::new();
        let mut rotz = std::f32::consts::PI;

        for i in 0..COUNT - 2 {
            let sequence = commands
                .add_motion(
                    sector
                        .transform()
                        .to_translation_x(target)
                        .with_ease(ease::cubic::ease_in)
                        .animate(0.8),
                )
                .add_motion(
                    sector
                        .transform()
                        .to_translation_y(MAX - SPACING * (i + 1) as f32)
                        .with_ease(ease::cubic::ease_out)
                        .animate(0.4),
                )
                .build()
                .chain();

            let rot_sequence = commands.play_motion(
                sector
                    .transform()
                    .to_rotation(Quat::from_rotation_z(rotz))
                    .with_ease(ease::cubic::ease_in_out)
                    .animate(1.2),
            );

            sequences.push([sequence, rot_sequence].all());
            target *= -1.0;
            rotz += std::f32::consts::PI;
        }

        // Last animation does not consists of y movement
        sequences.push(
            commands
                .add_motion(
                    sector
                        .transform()
                        .to_translation_x(target)
                        .with_ease(ease::cubic::ease_in_out)
                        .animate(0.8),
                )
                .add_motion(
                    sector
                        .transform()
                        .to_rotation(Quat::from_rotation_z(rotz))
                        .with_ease(ease::cubic::ease_in_out)
                        .animate(0.8),
                )
                .build()
                .all(),
        );
        target *= -1.0;
        rotz += std::f32::consts::PI;

        for i in 0..COUNT - 3 {
            let sequence = commands
                .add_motion(
                    sector
                        .transform()
                        .to_translation_y(MAX - SPACING * (COUNT - i - 3) as f32)
                        .with_ease(ease::cubic::ease_in)
                        .animate(0.4),
                )
                .add_motion(
                    sector
                        .transform()
                        .to_translation_x(target)
                        .with_ease(ease::cubic::ease_out)
                        .animate(0.8),
                )
                .build()
                .chain();

            let rot_sequence = commands.play_motion(
                sector
                    .transform()
                    .to_rotation(Quat::from_rotation_z(rotz))
                    .with_ease(ease::cubic::ease_in_out)
                    .animate(1.2),
            );

            sequences.push([sequence, rot_sequence].all());
            target *= -1.0;
            rotz += std::f32::consts::PI;
        }

        // Last animation does not consists of x movement
        sequences.push(
            commands.play_motion(
                sector
                    .transform()
                    .to_translation_y(MAX)
                    .with_ease(ease::cubic::ease_in_out)
                    .animate(0.5),
            ),
        );

        commands.spawn(SequenceBundle {
            sequence: sequences.chain(),
            ..default()
        });
    }
}

fn follow_mouse(
    q_window: Query<&Window, With<PrimaryWindow>>,
    mut q_marked: Query<&mut Transform, With<Marked>>,
) {
    let Ok(window) = q_window.get_single() else {
        return;
    };

    let Some(mut cursor_position) = window.cursor_position() else {
        return;
    };
    let width = window.width();
    let height = window.height();

    cursor_position.y = -cursor_position.y;
    cursor_position += Vec2::new(-width, height) * 0.5;

    for mut transform in q_marked.iter_mut() {
        transform.translation.x = cursor_position.x;
        transform.translation.y = cursor_position.y;
    }
}

fn cascade_settings(
    mut q_cascade: Query<&mut radiance_cascades::RadianceCascadesConfig>,
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
) {
    const SPEED: f32 = 8.0;
    let Ok(mut config) = q_cascade.get_single_mut() else {
        return;
    };

    let speed = SPEED
        * match keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            true => 32.0,
            false => 1.0,
        };

    if keyboard.pressed(KeyCode::ArrowUp) {
        let interval = config.get_interval();
        config.set_interval(interval + time.delta_seconds() * speed);
    }
    if keyboard.pressed(KeyCode::ArrowDown) {
        let interval = config.get_interval();
        config.set_interval(interval - time.delta_seconds() * speed);
    }
}

fn timeline_movement(
    mut q_timelines: Query<(&Sequence, &mut SequenceController)>,
    time: Res<Time>,
) {
    for (sequence, mut sequence_time) in q_timelines.iter_mut() {
        sequence_time.target_time =
            (sequence_time.target_time + time.delta_seconds()) % sequence.duration();
    }
}
