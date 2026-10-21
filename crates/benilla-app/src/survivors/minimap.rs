//! The minimap, laid out as `Minimap.xml` lays out `MinimapCluster` at the top right: the zone line
//! on the border's top strip, the 140-unit map in its ring with the zoom buttons, the minimize
//! button, and `GameTime.xml`'s day clock.
//!
//! Bevy UI clips only to rectangles, so the round map is composited here each frame: the ADT tiles
//! through `md5translate.trs` (north up, 533.33 yd per 256-px tile), masked by `MinimapMask.blp`,
//! then the player arrow, then the red `ObjectIcons` cell per live enemy, the dot the reference
//! draws for a tracked creature. Sizes are the engine minimap's ([`crate::minimap`]), measured
//! against its 140.8-px blip basis.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use benilla_assets::{LockRecover, MapCatalogRes, WorldAssets};
use benilla_formats::{MinimapTranslate, tile_to_world, world_to_tile};
use benilla_world::world_map::CurrentMap;
use benilla_world::world_point::WorldPoint;

use super::Run;
use super::enemies::Enemy;
use super::hero::Hero;
use super::hud::UiArt;
use crate::area::AreaTableRes;

/// The composite's edge in texels; the 140-unit widget is ~200 px at 1080p.
const SIDE: usize = 192;
const TILE_YARDS: f32 = 533.333_3;
const TILE_PX: usize = 256;
const CHUNK_YARDS: f32 = TILE_YARDS / 16.0;
/// The outdoor zoom table (`0x8116d0`): view diameter in chunks per `Minimap:SetZoom` index.
const ZOOM_CHUNKS: [f32; 6] = [14.0, 12.0, 10.0, 8.0, 6.0, 4.0];
/// Fully in: the packs that matter close in from ~30 yd, which the wider zooms crowd to the middle.
const START_ZOOM: usize = 5;

/// The engine minimap's blip basis: the stock widget at 140.8 px on the 1024×768 screen.
const BLIP_BASIS_PX: f32 = 140.8;
const DOT_PX: f32 = 8.0;
/// `ObjectIcons.blp` cell 1 (left, right, top, bottom): a unit passing creature tracking.
const TRACKED_UNIT_CELL: [f32; 4] = [0.25, 0.5, 0.0, 0.25];
const ARROW_PX: f32 = 33.6;
const ARROW_OFFSET_PX: Vec2 = Vec2::new(0.51, -1.73);

/// `GAMETIME_DAWN` and `GAMETIME_DUSK`, minutes into the day.
const DAWN: u32 = 5 * 60 + 30;
const DUSK: u32 = 21 * 60;

/// `FactionGroup.dbc` masks.
const GROUP_ALLIANCE: u32 = 2;
const GROUP_HORDE: u32 = 4;

#[derive(Component)]
pub(super) struct ZoneText;

/// The map body: hidden by the minimize button with everything in its ring.
#[derive(Component)]
pub(super) struct MapBody;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(super) enum MapButton {
    ZoomIn,
    ZoomOut,
    Toggle,
}

/// Decoded RGBA8 art the composite samples.
struct Rgba {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Rgba {
    fn decode(assets: &mut WorldAssets, path: &str) -> Option<Self> {
        let (w, h, px) = assets.decode_rgba(path)?;
        Some(Self {
            w: w as usize,
            h: h as usize,
            px,
        })
    }

    /// Bilinear RGBA at texel-space `(x, y)`, clamped to the edge.
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let x = (x - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (y - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| {
            let i = (y * self.w + x) * 4;
            self.px
                .get(i..i + 4)
                .and_then(|p| <[u8; 4]>::try_from(p).ok())
                .map_or([0.0; 4], |p| p.map(f32::from))
        };
        bilerp([at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1)], fx, fy)
    }
}

/// Bilinear mix of the four corners `[top-left, top-right, bottom-left, bottom-right]`.
fn bilerp<const N: usize>(corners: [[f32; N]; 4], fx: f32, fy: f32) -> [f32; N] {
    let [a, b, c, d] = corners;
    let mut out = [0.0; N];
    for ((o, (a, b)), (c, d)) in out.iter_mut().zip(a.iter().zip(&b)).zip(c.iter().zip(&d)) {
        let top = a + (b - a) * fx;
        let bottom = c + (d - c) * fx;
        *o = top + (bottom - top) * fy;
    }
    out
}

struct Art {
    translate: MinimapTranslate,
    mask: Option<Rgba>,
    arrow: Option<Rgba>,
    icons: Option<Rgba>,
}

#[derive(Resource)]
pub(super) struct MiniMap {
    zoom: usize,
    shown: bool,
    image: Handle<Image>,
    art: Option<Art>,
    /// Decoded 256² tiles by ADT index, `None` where no art was authored, for the map `dir` names.
    tiles: HashMap<(u32, u32), Option<Vec<u8>>>,
    dir: String,
    buf: Vec<u8>,
}

impl MiniMap {
    pub(super) fn new(images: &mut Assets<Image>) -> Self {
        let image = Image::new(
            Extent3d {
                width: SIDE as u32,
                height: SIDE as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![0; SIDE * SIDE * 4],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        Self {
            zoom: START_ZOOM,
            shown: true,
            image: images.add(image),
            art: None,
            tiles: HashMap::new(),
            dir: String::new(),
            buf: vec![0; SIDE * SIDE * 4],
        }
    }

    fn view_radius(&self) -> f32 {
        let chunks = ZOOM_CHUNKS
            .get(self.zoom)
            .or_else(|| ZOOM_CHUNKS.last())
            .copied()
            .unwrap_or_default();
        chunks * 0.5 * CHUNK_YARDS
    }
}

/// `MinimapCluster` and its children, in cluster-local units (top-left origin) from `Minimap.xml`.
pub(super) fn spawn_cluster(root: &mut ChildSpawnerCommands, font: &Handle<Font>, map: &MiniMap) {
    let at = |left: f32, top: f32, w: f32, h: f32| Node {
        position_type: PositionType::Absolute,
        left: Val::Px(left),
        top: Val::Px(top),
        width: Val::Px(w),
        height: Val::Px(h),
        ..default()
    };
    let art = |path: &'static str, coords: [f32; 4]| {
        (
            ImageNode {
                color: Color::NONE,
                ..default()
            },
            UiArt::crop(path, coords),
        )
    };
    root.spawn(Node {
        position_type: PositionType::Absolute,
        right: Val::Px(0.0),
        top: Val::Px(0.0),
        width: Val::Px(192.0),
        height: Val::Px(192.0),
        ..default()
    })
    .with_children(|cluster| {
        cluster.spawn((
            at(0.0, 0.0, 192.0, 32.0),
            art(
                "Interface\\Minimap\\UI-Minimap-Border",
                [0.25, 1.0, 0.0, 0.125],
            ),
        ));
        cluster
            .spawn(at(29.0, 7.0, 128.0, 12.0))
            .with_children(|zone| {
                zone.spawn((
                    ZoneText,
                    Text::new(""),
                    TextFont {
                        font: font.clone(),
                        font_size: 12.0,
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.82, 0.0)),
                    TextShadow {
                        offset: Vec2::new(1.0, 1.0),
                        color: Color::BLACK,
                    },
                    TextLayout::new(Justify::Center, LineBreak::NoWrap),
                    Node {
                        width: Val::Percent(100.0),
                        ..default()
                    },
                ));
            });
        cluster
            .spawn((MapBody, at(35.0, 22.0, 140.0, 140.0)))
            .with_children(|body| {
                body.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    ImageNode::new(map.image.clone()),
                ));
                // `MinimapBackdrop`: the ring, 192² at the cluster's (0, 20).
                body.spawn((
                    at(-35.0, -2.0, 192.0, 192.0),
                    art(
                        "Interface\\Minimap\\UI-Minimap-Border",
                        [0.25, 1.0, 0.125, 0.875],
                    ),
                ));
                body.spawn((
                    MapButton::ZoomIn,
                    Button,
                    at(122.0, 91.0, 32.0, 32.0),
                    ImageNode::default(),
                ));
                body.spawn((
                    MapButton::ZoomOut,
                    Button,
                    at(96.0, 119.0, 32.0, 32.0),
                    ImageNode::default(),
                ));
            });
        // `GameTimeFrame`, 50² at the cluster's top right, offset (4, -19).
        let tod = if (DAWN..DUSK).contains(&super::hero::MINUTE_OF_DAY) {
            0.0
        } else {
            0.5
        };
        cluster.spawn((
            at(146.0, 19.0, 50.0, 50.0),
            art(
                "Interface\\Minimap\\UI-TOD-Indicator",
                [tod, tod + 50.0 / 128.0, 0.0, 50.0 / 64.0],
            ),
        ));
        cluster.spawn((
            MapButton::Toggle,
            Button,
            at(161.0, -3.0, 32.0, 32.0),
            ImageNode::default(),
        ));
    });
}

/// The zoom and minimize buttons: their `Up`/`Down`/`Disabled` art, and a press acts.
pub(super) fn map_buttons(
    mut map: ResMut<MiniMap>,
    mut buttons: Query<(Ref<Interaction>, &MapButton, &mut ImageNode)>,
    mut bodies: Query<&mut Node, With<MapBody>>,
    assets: Option<ResMut<WorldAssets>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut assets) = assets else { return };
    for (interaction, button, mut img) in &mut buttons {
        let pressed = *interaction == Interaction::Pressed;
        if pressed && interaction.is_changed() {
            match button {
                MapButton::ZoomIn => map.zoom = (map.zoom + 1).min(ZOOM_CHUNKS.len() - 1),
                MapButton::ZoomOut => map.zoom = map.zoom.saturating_sub(1),
                MapButton::Toggle => map.shown = !map.shown,
            }
        }
        let disabled = match button {
            MapButton::ZoomIn => map.zoom == ZOOM_CHUNKS.len() - 1,
            MapButton::ZoomOut => map.zoom == 0,
            MapButton::Toggle => false,
        };
        let stem = match button {
            MapButton::ZoomIn => "Interface\\Minimap\\UI-Minimap-ZoomInButton",
            MapButton::ZoomOut => "Interface\\Minimap\\UI-Minimap-ZoomOutButton",
            MapButton::Toggle => "Interface\\Buttons\\UI-Panel-MinimizeButton",
        };
        let state = match (disabled, pressed) {
            (true, _) => "Disabled",
            (false, true) => "Down",
            (false, false) => "Up",
        };
        if let Some(h) = assets.sprite_texture(&format!("{stem}-{state}"), &mut images) {
            if img.image != h {
                img.image = h;
            }
        }
    }
    let display = if map.shown {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut bodies {
        if node.display != display {
            node.display = display;
        }
    }
}

/// `Minimap_Update`: `GetMinimapZoneText` (the subzone, else the zone) in `GetZonePVPInfo`'s
/// colour, the zone's owner against the hero's faction.
pub(super) fn zone_text(
    world: WorldPoint,
    areas: Option<Res<AreaTableRes>>,
    run: Res<Run>,
    mut texts: Query<(&mut Text, &mut TextColor), With<ZoneText>>,
) {
    let Some(areas) = areas else { return };
    let Some(leaf) = world.area() else { return };
    let Some(row) = areas.0.get(leaf) else { return };
    let zone = if row.zone_id == 0 { leaf } else { row.zone_id };
    let name = if leaf == zone {
        areas.0.name(zone).unwrap_or_default().to_string()
    } else {
        row.name.clone()
    };
    let (friend, enemy) = match run.race {
        1 | 3 | 4 | 7 => (GROUP_ALLIANCE, GROUP_HORDE),
        _ => (GROUP_HORDE, GROUP_ALLIANCE),
    };
    let owner = areas.0.get(zone).map_or(0, |r| r.faction_group_mask);
    let color = if row.flags & 0x80 != 0 || owner & enemy != 0 {
        Color::srgb(1.0, 0.1, 0.1)
    } else if owner & friend != 0 {
        Color::srgb(0.1, 1.0, 0.1)
    } else {
        Color::srgb(1.0, 0.7, 0.0)
    };
    for (mut text, mut tc) in &mut texts {
        if text.0 != name {
            text.0 = name.clone();
        }
        if tc.0 != color {
            tc.0 = color;
        }
    }
}

/// Recomposite the map around the hero: tiles, mask, arrow, then the enemy dots.
#[allow(clippy::too_many_arguments)]
pub(super) fn compose(
    mut map: ResMut<MiniMap>,
    assets: Option<ResMut<WorldAssets>>,
    catalog: Option<Res<MapCatalogRes>>,
    current: Option<Res<CurrentMap>>,
    phase: Res<State<super::Phase>>,
    hero: Query<&Transform, With<Hero>>,
    enemies: Query<(&Transform, &Enemy)>,
    mut images: ResMut<Assets<Image>>,
) {
    if *phase.get() == super::Phase::Menu || !map.shown {
        return;
    }
    let (Some(mut assets), Some(catalog), Some(current)) = (assets, catalog, current) else {
        return;
    };
    let Ok(hero) = hero.single() else { return };
    let Some(dir) = catalog.0.directory(current.0).map(str::to_string) else {
        return;
    };
    let map = &mut *map;
    if map.dir != dir {
        map.tiles.clear();
        map.dir.clone_from(&dir);
    }
    if map.art.is_none() {
        let translate = {
            let mut chain = assets.chain.lock_recover();
            benilla_formats::load_minimap_translate(&mut chain)
        };
        let Ok(translate) = translate else {
            warn!("survivors: md5translate.trs failed, no minimap");
            map.shown = false;
            return;
        };
        map.art = Some(Art {
            translate,
            mask: Rgba::decode(&mut assets, "Textures\\MinimapMask"),
            arrow: Rgba::decode(&mut assets, crate::minimap::blips::PLAYER_ARROW_TEXTURE),
            icons: Rgba::decode(&mut assets, "Interface\\Minimap\\ObjectIcons"),
        });
    }
    let art = map.art.as_ref().expect("art loaded above");

    // The hero in WoW coordinates; screen right is east (-y), screen down south (-x).
    let p = hero.translation;
    let (wx, wy) = (-p.z, -p.x);
    let radius = map.view_radius();
    let yd_per_px = 2.0 * radius / SIDE as f32;

    // The 2×2 tile window under the view: its diameter (at most 14 chunks) is under one tile.
    let (tx0, ty0) = world_to_tile(wx + radius + 1.0, wy + radius + 1.0);
    for ty in ty0..=ty0 + 1 {
        for tx in tx0..=tx0 + 1 {
            map.tiles.entry((tx, ty)).or_insert_with(|| {
                let hash = art.translate.tile(&dir, tx, ty)?;
                let tile = Rgba::decode(&mut assets, &format!("textures\\Minimap\\{hash}"))?;
                (tile.w == TILE_PX && tile.h == TILE_PX).then_some(tile.px)
            });
        }
    }
    let window: [[Option<&[u8]>; 2]; 2] = [0, 1].map(|j| {
        [0, 1].map(|i| {
            map.tiles
                .get(&(tx0 + i, ty0 + j))
                .and_then(|t| t.as_deref())
        })
    });
    // Window-local tile texels: x grows east from the window's west edge, y south from its north.
    let (north, west) = tile_to_world(tx0, ty0);
    let tex_per_yd = TILE_PX as f32 / TILE_YARDS;
    let origin = Vec2::new((west - wy) * tex_per_yd, (north - wx) * tex_per_yd);
    let tex_per_px = yd_per_px * tex_per_yd;
    let texel = |x: i32, y: i32| -> [f32; 3] {
        let (cx, cy) = (x.div_euclid(TILE_PX as i32), y.div_euclid(TILE_PX as i32));
        if !(0..2).contains(&cx) || !(0..2).contains(&cy) {
            return [0.0; 3];
        }
        let tile = window
            .get(cy as usize)
            .and_then(|row| row.get(cx as usize))
            .copied()
            .flatten();
        let Some(px) = tile else {
            return [0.0; 3];
        };
        let (lx, ly) = (x.rem_euclid(TILE_PX as i32), y.rem_euclid(TILE_PX as i32));
        let i = (ly as usize * TILE_PX + lx as usize) * 4;
        px.get(i..i + 3)
            .and_then(|p| <[u8; 3]>::try_from(p).ok())
            .map_or([0.0; 3], |p| p.map(f32::from))
    };

    let half = SIDE as f32 / 2.0;
    let buf = &mut map.buf;
    let (pixels, _) = buf.as_chunks_mut::<4>();
    for (v, row) in pixels.chunks_exact_mut(SIDE).enumerate() {
        for (u, pix) in row.iter_mut().enumerate() {
            let mask = match &art.mask {
                Some(m) => {
                    let (mx, my) = (u * m.w / SIDE, v * m.h / SIDE);
                    let i = (my * m.w + mx) * 4;
                    m.px.get(i..i + 4)
                        .and_then(|p| <[u8; 4]>::try_from(p).ok())
                        .map_or(0, |[r, _, _, a]| r.min(a))
                }
                None => {
                    let d = Vec2::new(u as f32 + 0.5 - half, v as f32 + 0.5 - half).length();
                    ((half - d).clamp(0.0, 1.0) * 255.0) as u8
                }
            };
            if mask == 0 {
                *pix = [0; 4];
                continue;
            }
            let g = origin + (Vec2::new(u as f32, v as f32) + 0.5 - half) * tex_per_px - 0.5;
            let (x0, y0) = (g.x.floor() as i32, g.y.floor() as i32);
            let (fx, fy) = (g.x - x0 as f32, g.y - y0 as f32);
            let [r, g, b] = bilerp(
                [
                    texel(x0, y0),
                    texel(x0 + 1, y0),
                    texel(x0, y0 + 1),
                    texel(x0 + 1, y0 + 1),
                ],
                fx,
                fy,
            );
            *pix = [r as u8, g as u8, b as u8, mask];
        }
    }

    let scale = SIDE as f32 / BLIP_BASIS_PX;
    let center = Vec2::splat(half);
    // The arrow turns with the hero's facing: counter-clockwise from north, as the yaw runs.
    if let Some(arrow) = &art.arrow {
        let yaw = hero.rotation.to_euler(EulerRot::YXZ).0;
        let (s, c) = yaw.sin_cos();
        let to_screen = |t: Vec2| Vec2::new(c * t.x + s * t.y, -s * t.x + c * t.y);
        let to_tex = |q: Vec2| Vec2::new(c * q.x - s * q.y, s * q.x + c * q.y);
        let size = ARROW_PX * scale;
        blit(
            buf,
            center + to_screen(ARROW_OFFSET_PX * scale),
            size,
            |q| {
                let t = to_tex(q) / size + 0.5;
                (0.0..1.0).contains(&t.x).then_some(())?;
                (0.0..1.0).contains(&t.y).then_some(())?;
                Some(arrow.sample(t.x * arrow.w as f32, t.y * arrow.h as f32))
            },
        );
    }
    if let Some(icons) = &art.icons {
        let size = DOT_PX * scale;
        let [l, r, t, b] = TRACKED_UNIT_CELL;
        for (tf, enemy) in &enemies {
            if enemy.dead {
                continue;
            }
            let off = Vec2::new(tf.translation.x - p.x, tf.translation.z - p.z);
            if off.length() > radius {
                continue;
            }
            blit(buf, center + off / yd_per_px, size, |q| {
                let f = q / size + 0.5;
                let x = (l + (r - l) * f.x) * icons.w as f32;
                let y = (t + (b - t) * f.y) * icons.h as f32;
                Some(icons.sample(x, y))
            });
        }
    }

    if let Some(image) = images.get_mut(&map.image) {
        if let Some(data) = image.data.as_mut() {
            data.copy_from_slice(&map.buf);
        }
    }
}

/// Alpha-blend a `size`-px square centred at `at`; `shade` maps a pixel's offset from the centre
/// to its RGBA. The map's own alpha (the mask) is kept.
fn blit(buf: &mut [u8], at: Vec2, size: f32, shade: impl Fn(Vec2) -> Option<[f32; 4]>) {
    let half = size / 2.0;
    let x_lo = (at.x - half).floor().max(0.0) as usize;
    let y_lo = (at.y - half).floor().max(0.0) as usize;
    let x_hi = ((at.x + half).ceil() as usize).min(SIDE);
    let y_hi = ((at.y + half).ceil() as usize).min(SIDE);
    for y in y_lo..y_hi {
        for x in x_lo..x_hi {
            let o = (y * SIDE + x) * 4;
            let Some(pix) = buf
                .get_mut(o..o + 4)
                .and_then(|p| <&mut [u8; 4]>::try_from(p).ok())
            else {
                continue;
            };
            if pix[3] == 0 {
                continue;
            }
            let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - at;
            let Some(src) = shade(q) else { continue };
            let a = src[3] / 255.0;
            for (dst, src) in pix.iter_mut().take(3).zip(src) {
                let d = f32::from(*dst);
                *dst = (d + (src - d) * a) as u8;
            }
        }
    }
}
