//! Client-only park dressing. The road follows the shared course exactly;
//! scenery outside its shoulders has no authority over movement or combat.
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use riverside_common::{BikeState, RiversideCourse, RoadSection};

use crate::game::CourseVisual;

// Extend scenery behind the start so the rear-facing Gunner sees a park,
// rather than the end of the playable course. These are visual metres only.
const APPROACH_M: f32 = 240.0;
const RUNOUT_M: f32 = 120.0;

pub const SKY: Color = Color::srgb(0.70, 0.81, 0.88);

pub fn park_fog() -> DistanceFog {
    DistanceFog {
        color: SKY,
        falloff: FogFalloff::Linear {
            start: 100.0,
            end: 640.0,
        },
        ..default()
    }
}

fn matte(materials: &mut Assets<StandardMaterial>, color: Color) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: color,
        perceptual_roughness: 0.93,
        ..default()
    })
}

fn object(
    commands: &mut Commands,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
    position: Vec3,
    scale: Vec3,
    rotation: Quat,
) {
    commands.spawn((
        CourseVisual,
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(position)
            .with_scale(scale)
            .with_rotation(rotation),
    ));
}

/// Small disconnected geometry is batched by material, avoiding an entity per
/// grass blade, water glint, or painted dash on the WebGL2 client.
#[derive(Default)]
struct Surface {
    vertices: Vec<[f32; 3]>,
    indices: Vec<u32>,
}
impl Surface {
    fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
        let i = self.vertices.len() as u32;
        self.vertices
            .extend([a.to_array(), b.to_array(), c.to_array(), d.to_array()]);
        self.indices.extend([i, i + 1, i + 2, i, i + 2, i + 3]);
    }
    fn finish(
        self,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        material: Handle<StandardMaterial>,
    ) {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.vertices)
        .with_inserted_indices(Indices::U32(self.indices));
        mesh.compute_normals();
        commands.spawn((
            CourseVisual,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
        ));
    }
}

fn route(course: &RiversideCourse, distance: f32, lateral: f32, height: f32) -> Vec3 {
    let point = course.point_at(distance.clamp(0.0, course.length_m()));
    let extension = distance - distance.clamp(0.0, course.length_m());
    Vec3::new(
        point.center_x + lateral * point.yaw.cos() + extension * point.yaw.sin(),
        height,
        point.center_z + lateral * point.yaw.sin() - extension * point.yaw.cos(),
    )
}

fn ribbon(
    course: &RiversideCourse,
    surface: &mut Surface,
    start: f32,
    end: f32,
    left: f32,
    right: f32,
    height: f32,
) {
    let steps = ((end - start) / 0.65).ceil() as usize;
    for i in 0..steps {
        let a = start + (end - start) * i as f32 / steps as f32;
        let b = start + (end - start) * (i + 1) as f32 / steps as f32;
        surface.quad(
            route(course, a, left, height),
            route(course, a, right, height),
            route(course, b, right, height),
            route(course, b, left, height),
        );
    }
}

// Deterministic variation keeps the park stable across reloads, without RNG dependencies.
fn variation(i: usize, salt: usize) -> f32 {
    ((i.wrapping_mul(1664525)
        .wrapping_add(salt.wrapping_mul(1013904223)))
        % 1009) as f32
        / 1009.0
}

pub fn spawn_park(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    course: &RiversideCourse,
) {
    let length = course.length_m();
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let crown = meshes.add(
        Sphere::new(1.0)
            .mesh()
            .ico(2)
            .expect("valid tree subdivision"),
    );
    let trunk = meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(7));
    let grass = matte(materials, Color::srgb(0.28, 0.37, 0.13));
    let bank = matte(materials, Color::srgb(0.37, 0.42, 0.21));
    let concrete = matte(materials, Color::srgb(0.49, 0.48, 0.42));
    let asphalt = matte(materials, Color::srgb(0.19, 0.205, 0.21));
    let white = matte(materials, Color::srgb(0.85, 0.85, 0.73));
    let yellow = matte(materials, Color::srgb(0.88, 0.64, 0.18));
    let metal = matte(materials, Color::srgb(0.18, 0.23, 0.20));
    let bark = matte(materials, Color::srgb(0.25, 0.19, 0.12));
    let wood = matte(materials, Color::srgb(0.43, 0.29, 0.16));
    let leaves = [
        matte(materials, Color::srgb(0.16, 0.29, 0.085)),
        matte(materials, Color::srgb(0.24, 0.36, 0.105)),
        matte(materials, Color::srgb(0.32, 0.41, 0.14)),
    ];
    let min_x = course
        .pieces()
        .iter()
        .map(|p| p.root_x)
        .fold(0.0_f32, f32::min);
    let max_x = course
        .pieces()
        .iter()
        .map(|p| p.root_x)
        .fold(0.0_f32, f32::max);
    let start_z = BikeState::default().z + APPROACH_M + 80.0;
    let end_z = course.point_at(length).center_z - 300.0;
    let middle_z = (start_z + end_z) * 0.5;
    let depth = start_z - end_z;
    let shore = min_x - 24.0;
    let levee = max_x + 32.0;

    // The lawn ends at the bank, so it never covers the lower water plane.
    object(
        commands,
        &cube,
        &grass,
        Vec3::new((shore + levee) * 0.5, -0.32, middle_z),
        Vec3::new(levee - shore, 0.5, depth),
        Quat::IDENTITY,
    );
    let mut banks = Surface::default();
    for (x0, y0, x1, y1) in [
        (shore - 8.0, -1.7, shore, -0.07),
        (shore - 155.0, -0.2, shore - 146.0, -1.7),
        (levee, -0.07, levee + 19.0, 5.0),
    ] {
        banks.quad(
            Vec3::new(x0, y0, start_z),
            Vec3::new(x1, y1, start_z),
            Vec3::new(x1, y1, end_z),
            Vec3::new(x0, y0, end_z),
        );
    }
    banks.finish(commands, meshes, bank);
    object(
        commands,
        &cube,
        &grass,
        Vec3::new(shore - 192.0, -0.5, middle_z),
        Vec3::new(75.0, 0.6, depth),
        Quat::IDENTITY,
    );
    object(
        commands,
        &cube,
        &concrete,
        Vec3::new(levee + 20.0, 5.05, middle_z),
        Vec3::new(2.0, 0.25, depth),
        Quat::IDENTITY,
    );
    let water = materials.add(StandardMaterial {
        base_color: Color::srgb(0.24, 0.39, 0.40),
        metallic: 0.18,
        perceptual_roughness: 0.34,
        ..default()
    });
    object(
        commands,
        &cube,
        &water,
        Vec3::new(shore - 77.0, -1.8, middle_z),
        Vec3::new(140.0, 0.12, depth),
        Quat::IDENTITY,
    );
    let mut ripples = Surface::default();
    for i in 0..950 {
        let x = shore - 10.0 - variation(i, 2) * 133.0;
        let z = end_z + variation(i, 3) * depth;
        let width = 0.8 + variation(i, 4) * 5.0;
        ripples.quad(
            Vec3::new(x, -1.732, z),
            Vec3::new(x + width, -1.732, z),
            Vec3::new(x + width, -1.732, z - 0.10),
            Vec3::new(x, -1.732, z - 0.10),
        );
    }
    let ripple_material = matte(materials, Color::srgb(0.38, 0.51, 0.50));
    ripples.finish(commands, meshes, ripple_material);

    // Keep the shared four-metre pavement, shoulders and visible boundary.
    let mut road = Surface::default();
    let mut shoulders = Surface::default();
    let mut paint = Surface::default();
    let mut centerline = Surface::default();
    ribbon(
        course,
        &mut road,
        -APPROACH_M,
        length + RUNOUT_M,
        -2.0,
        2.0,
        0.0,
    );
    for side in [-1.0, 1.0] {
        ribbon(
            course,
            &mut shoulders,
            -APPROACH_M,
            length + RUNOUT_M,
            side * 2.15 - 0.15,
            side * 2.15 + 0.15,
            -0.012,
        );
        ribbon(
            course,
            &mut paint,
            -APPROACH_M,
            length + RUNOUT_M,
            side * 1.92 - 0.035,
            side * 1.92 + 0.035,
            0.012,
        );
    }
    for i in 0..((length + APPROACH_M + RUNOUT_M) / 4.0) as usize {
        let d = i as f32 * 4.0 - APPROACH_M;
        ribbon(course, &mut centerline, d, d + 1.6, -0.045, 0.045, 0.015);
    }
    road.finish(commands, meshes, asphalt);
    shoulders.finish(commands, meshes, concrete.clone());
    paint.finish(commands, meshes, white.clone());
    centerline.finish(commands, meshes, yellow);

    // No rails: the lawn is rideable (slowly). Shortcut chords, their ramps
    // and the creeks that cut across them come from the shared course data.
    spawn_shortcuts(commands, meshes, materials, course, &cube);

    // Broad, irregular canopies and open lawns: a riverside park, not a forest.
    for i in 0..((length + APPROACH_M + 36.0) / 12.0) as usize {
        let d = i as f32 * 12.0 - APPROACH_M + 16.0;
        for side in [-1.0, 1.0] {
            if side < 0.0 && i % 3 != 0 {
                continue;
            }
            let setback = if course.section_at(d.max(0.0)) == RoadSection::BridgeUnderpass {
                14.0
            } else {
                8.0
            };
            let offset = side * (setback + variation(i, 5) * 4.0);
            let p = route(course, d, offset, -0.05);
            // Guard against a decorative trunk landing on another path or creek.
            if course.nearest_road_point(p.x, p.z).distance_m < 4.0
                || course.water().iter().any(|creek| creek.contains(p.x, p.z))
            {
                continue;
            }
            let size = 0.9 + variation(i, 6) * 0.55;
            object(
                commands,
                &trunk,
                &bark,
                p + Vec3::Y * 1.8 * size,
                Vec3::new(0.19 * size, 3.6 * size, 0.19 * size),
                Quat::IDENTITY,
            );
            for (j, offset) in [
                Vec3::new(0.0, 4.3, 0.0),
                Vec3::new(-1.1, 3.7, 0.2),
                Vec3::new(1.1, 3.9, -0.3),
                Vec3::new(0.1, 3.8, 1.1),
            ]
            .into_iter()
            .enumerate()
            {
                object(
                    commands,
                    &crown,
                    &leaves[(i + j) % 3],
                    p + offset * size,
                    Vec3::new(2.05, 1.5, 1.8) * size,
                    Quat::from_rotation_y(variation(i + j, 7) * 6.0),
                );
            }
        }
    }

    // Lamps and benches on the landward verge, outside the playable width.
    for i in 0..((length + APPROACH_M + 36.0) / 30.0) as usize {
        let d = i as f32 * 30.0 - APPROACH_M + 15.0;
        if course.section_at(d.max(0.0)) == RoadSection::BridgeUnderpass {
            continue;
        }
        let p = route(course, d, 3.5, 0.0);
        if course.nearest_road_point(p.x, p.z).distance_m < 3.0 {
            continue;
        }
        let yaw = Quat::from_rotation_y(-course.point_at(d.max(0.0)).yaw);
        object(
            commands,
            &trunk,
            &metal,
            p + Vec3::Y * 2.8,
            Vec3::new(0.055, 5.6, 0.055),
            Quat::IDENTITY,
        );
        object(
            commands,
            &cube,
            &metal,
            p + yaw * Vec3::new(-0.42, 5.55, 0.0),
            Vec3::new(0.95, 0.08, 0.08),
            yaw,
        );
        object(
            commands,
            &cube,
            &white,
            p + yaw * Vec3::new(-0.83, 5.51, 0.0),
            Vec3::new(0.52, 0.10, 0.27),
            yaw,
        );
        if i % 3 == 0 {
            let p = route(course, d + 6.0, 4.6, 0.0);
            object(
                commands,
                &cube,
                &concrete,
                p + Vec3::new(0.0, 0.015, 0.0),
                Vec3::new(2.5, 0.06, 3.1),
                yaw,
            );
            for z in [-0.75, 0.75] {
                object(
                    commands,
                    &cube,
                    &metal,
                    p + yaw * Vec3::new(0.0, 0.23, z),
                    Vec3::new(0.5, 0.46, 0.075),
                    yaw,
                );
            }
            for x in [-0.22, 0.0, 0.22] {
                object(
                    commands,
                    &cube,
                    &wood,
                    p + yaw * Vec3::new(x, 0.49, 0.0),
                    Vec3::new(0.18, 0.075, 1.95),
                    yaw,
                );
            }
            for y in [0.76, 0.98] {
                object(
                    commands,
                    &cube,
                    &wood,
                    p + yaw * Vec3::new(0.32, y, 0.0),
                    Vec3::new(0.07, 0.17, 1.95),
                    yaw,
                );
            }
        }
    }

    // Riverbank reeds: crossed, opaque blades batched into one mesh.
    let mut reeds = Surface::default();
    for i in 0..1600 {
        let p = Vec3::new(
            shore - 1.0 - variation(i, 11) * 4.0,
            -0.45,
            end_z + variation(i, 12) * depth,
        );
        let h = 0.4 + variation(i, 13) * 0.8;
        for axis in [Vec3::X, Vec3::Z] {
            let a = p - axis * 0.15;
            let b = p + axis * 0.15;
            reeds.quad(
                a,
                b,
                p + Vec3::Y * h + axis * 0.04,
                p + Vec3::Y * h - axis * 0.04,
            );
            reeds.quad(
                b,
                a,
                p + Vec3::Y * h - axis * 0.04,
                p + Vec3::Y * h + axis * 0.04,
            );
        }
    }
    reeds.finish(commands, meshes, leaves[2].clone());

    // Low city silhouettes and wooded hills across the river. Deliberately
    // fictional, not a claim to reconstruct the supplied panorama's buildings.
    let city = [
        matte(materials, Color::srgb(0.56, 0.59, 0.58)),
        matte(materials, Color::srgb(0.64, 0.63, 0.57)),
        matte(materials, Color::srgb(0.46, 0.53, 0.55)),
    ];
    let windows = matte(materials, Color::srgb(0.28, 0.37, 0.39));
    for i in 0..64 {
        let z = start_z - i as f32 * depth / 64.0;
        let h = 9.0 + variation(i, 20) * 28.0;
        let w = 10.0 + variation(i, 21) * 12.0;
        let x = shore - 181.0 - variation(i, 22) * 24.0;
        object(
            commands,
            &cube,
            &city[i % 3],
            Vec3::new(x, h * 0.5, z),
            Vec3::new(15.0, h, w),
            Quat::IDENTITY,
        );
        object(
            commands,
            &cube,
            &city[(i + 1) % 3],
            Vec3::new(x, h + 1.0, z),
            Vec3::new(9.0, 2.0, w * 0.6),
            Quat::IDENTITY,
        );
        for floor in 1..(h / 3.2) as usize {
            object(
                commands,
                &cube,
                &windows,
                Vec3::new(x + 7.51, floor as f32 * 3.2, z),
                Vec3::new(0.03, 0.85, w * 0.8),
                Quat::IDENTITY,
            );
        }
    }
    let hills = matte(materials, Color::srgb(0.32, 0.43, 0.39));
    for i in 0..20 {
        object(
            commands,
            &crown,
            &hills,
            Vec3::new(shore - 340.0, 0.0, start_z - i as f32 * 110.0),
            Vec3::new(100.0, 35.0 + variation(i, 23) * 40.0, 105.0),
            Quat::IDENTITY,
        );
    }
}

fn spawn_shortcuts(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    course: &RiversideCourse,
    cube: &Handle<Mesh>,
) {
    if course.branches().is_empty() {
        return;
    }
    let asphalt = matte(materials, Color::srgb(0.21, 0.215, 0.22));
    let paint = matte(materials, Color::srgb(0.88, 0.64, 0.18));
    let ramp_material = matte(materials, Color::srgb(0.62, 0.58, 0.50));
    let stripe = matte(materials, Color::srgb(0.92, 0.42, 0.12));
    let creek_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.20, 0.40, 0.45),
        metallic: 0.2,
        perceptual_roughness: 0.25,
        ..default()
    });
    let bank = matte(materials, Color::srgb(0.36, 0.30, 0.20));

    let mut road = Surface::default();
    let mut dashes = Surface::default();
    for branch in course.branches() {
        let at = |distance: f32, lateral: f32, height: f32| {
            let point = branch.point_at(distance);
            Vec3::new(
                point.center_x + lateral * point.yaw.cos(),
                height,
                point.center_z + lateral * point.yaw.sin(),
            )
        };
        let mut distance = 0.0;
        while distance < branch.length_m() {
            let next = (distance + 0.65).min(branch.length_m());
            let mid = at((distance + next) * 0.5, 0.0, 0.0);
            // Leave the creek crossing unpaved so the water shows through.
            if !course
                .water()
                .iter()
                .any(|creek| creek.contains(mid.x, mid.z))
            {
                let half = riverside_common::BRANCH_HALF_WIDTH_M;
                road.quad(
                    at(distance, -half, 0.004),
                    at(distance, half, 0.004),
                    at(next, half, 0.004),
                    at(next, -half, 0.004),
                );
                if ((distance / 3.0) as usize).is_multiple_of(2) {
                    dashes.quad(
                        at(distance, -0.05, 0.016),
                        at(distance, 0.05, 0.016),
                        at(next, 0.05, 0.016),
                        at(next, -0.05, 0.016),
                    );
                }
            }
            distance = next;
        }
    }
    road.finish(commands, meshes, asphalt);
    dashes.finish(commands, meshes, paint);

    for creek in course.water() {
        let rotation = Quat::from_rotation_y(-creek.yaw);
        let centre = Vec3::new(creek.x, -0.03, creek.z);
        object(
            commands,
            cube,
            &creek_material,
            centre,
            Vec3::new(creek.half_width * 2.0, 0.04, creek.half_length * 2.0),
            rotation,
        );
        // Low muddy banks mark where the water begins.
        for side in [-1.0, 1.0] {
            object(
                commands,
                cube,
                &bank,
                centre + rotation * Vec3::new(0.0, 0.0, side * (creek.half_length + 0.15)),
                Vec3::new(creek.half_width * 2.0, 0.07, 0.3),
                rotation,
            );
        }
    }

    // Each ramp is a wedge rising to its forward (-Z) lip.
    let mut wedges = Surface::default();
    for ramp in course.ramps() {
        let patch = ramp.patch;
        let rotation = Quat::from_rotation_y(-patch.yaw);
        let corner = |x: f32, z: f32, y: f32| {
            Vec3::new(patch.x, 0.0, patch.z) + rotation * Vec3::new(x, y, z)
        };
        let (w, l, h) = (patch.half_width, patch.half_length, ramp.height);
        // Sloped deck, lip face, and both sides.
        wedges.quad(
            corner(-w, l, 0.0),
            corner(w, l, 0.0),
            corner(w, -l, h),
            corner(-w, -l, h),
        );
        wedges.quad(
            corner(-w, -l, h),
            corner(w, -l, h),
            corner(w, -l, 0.0),
            corner(-w, -l, 0.0),
        );
        wedges.quad(
            corner(w, l, 0.0),
            corner(w, -l, 0.0),
            corner(w, -l, h),
            corner(w, -l, h),
        );
        wedges.quad(
            corner(-w, -l, h),
            corner(-w, -l, 0.0),
            corner(-w, l, 0.0),
            corner(-w, l, 0.0),
        );
        for x in [-0.6, 0.6] {
            object(
                commands,
                cube,
                &stripe,
                corner(x * w, -l + 0.12, h + 0.01),
                Vec3::new(0.35, 0.02, 0.24),
                rotation,
            );
        }
    }
    wedges.finish(commands, meshes, ramp_material);
}
