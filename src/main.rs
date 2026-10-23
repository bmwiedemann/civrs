use civrs::assets::{AssetDir, Palette};
use civrs::platform::Backend;
use civrs::gfx::{Surface, TextStyle};
use std::path::{Path, PathBuf};

fn usage() -> ! {
    eprintln!(
        "civrs - Civilization (1991) reimplementation\n\
         usage: civrs [--assets DIR] [--scale N] [--fullscreen]\n\
                civrs --dump TARGET --out FILE.ppm [--assets DIR] [--zoom N]\n\
         dump targets: pic:NAME | sheet:NAME | fonts | pal:NAME | all-pics:DIR\n\
                       sound:ID[:LEVEL[:SECONDS]] (WAV) | sound-regs:ID[:LEVEL[:TICKS]] (OPL2 register log)"
    );
    std::process::exit(2);
}

struct Cli {
    assets: Option<PathBuf>,
    scale: Option<u32>,
    fullscreen: bool,
    dump: Option<String>,
    out: Option<PathBuf>,
    zoom: usize,
    skip_intro: bool,
    seed: Option<u64>,
}

fn parse_cli() -> Cli {
    let mut cli = Cli { assets: None, scale: None, fullscreen: false, dump: None, out: None, zoom: 1, skip_intro: false, seed: None };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--assets" => cli.assets = args.next().map(PathBuf::from),
            "--scale" => cli.scale = args.next().and_then(|s| s.parse().ok()),
            "--fullscreen" => cli.fullscreen = true,
            "--dump" => cli.dump = args.next(),
            "--out" => cli.out = args.next().map(PathBuf::from),
            "--skip-intro" => cli.skip_intro = true,
            "--seed" => cli.seed = args.next().and_then(|s| s.parse().ok()),
            "--zoom" => cli.zoom = args.next().and_then(|s| s.parse().ok()).unwrap_or(1),
            "-h" | "--help" => usage(),
            _ => {
                eprintln!("unknown argument {a}");
                usage();
            }
        }
    }
    cli
}

fn dump(assets: &AssetDir, target: &str, out: &Path, zoom: usize) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(spec) = target.strip_prefix("sound:") {
        return dump_sound(assets, spec, out, false);
    }
    if let Some(spec) = target.strip_prefix("sound-regs:") {
        return dump_sound(assets, spec, out, true);
    }
    let fonts = assets.load_fonts()?;
    if let Some(name) = target.strip_prefix("pic:") {
        let pic = assets.load_pic(name)?;
        let pal = pic.palette.unwrap_or_else(Palette::ega);
        let s = Surface::from_pic(&pic);
        s.write_ppm_scaled(&pal, out, zoom)?;
        println!("{name}: {}x{} depth {:?}", pic.width, pic.height, pic.depth);
    } else if let Some(name) = target.strip_prefix("sheet:") {
        // Sheet with a 16px grid overlay (colour 255) and cell coordinates in the tiny font.
        let pic = assets.load_pic(name)?;
        let mut pal = pic.palette.unwrap_or_else(Palette::ega);
        pal.0[255] = [255, 0, 255];
        pal.0[254] = [255, 255, 0];
        let src = Surface::from_pic(&pic);
        let zoom = zoom.max(4);
        // draw at zoom with grid lines on a bigger surface
        let mut big = Surface::new(src.w * zoom as i32, src.h * zoom as i32);
        for y in 0..big.h {
            for x in 0..big.w {
                big.put(x, y, src.get(x / zoom as i32, y / zoom as i32));
            }
        }
        for gx in (0..big.w).step_by(16 * zoom) {
            big.vline(gx, 0, big.h - 1, 255);
        }
        for gy in (0..big.h).step_by(16 * zoom) {
            big.hline(0, big.w - 1, gy, 255);
        }
        let st = TextStyle::new(1, 254);
        for cy in 0..(src.h / 16) {
            for cx in 0..(src.w / 16) {
                let label = format!("{cx},{cy}");
                civrs::gfx::draw_text(&mut big, &fonts, &st, cx * 16 * zoom as i32 + 2, cy * 16 * zoom as i32 + 2, &label);
            }
        }
        big.write_ppm(&pal, out)?;
    } else if let Some(name) = target.strip_prefix("idx:") {
        // Raw palette indices of a picture as an 8-bit PGM (for sprite analysis).
        let pic = assets.load_pic(name)?;
        let s = Surface::from_pic(&pic);
        let mut data = format!("P5\n{} {}\n255\n", s.w, s.h).into_bytes();
        for y in 0..s.h {
            for x in 0..s.w {
                data.push(s.get(x, y));
            }
        }
        std::fs::write(out, data)?;
    } else if target == "fonts" {
        let mut s = Surface::new(320, 400);
        s.fill(0);
        let mut pal = Palette::black();
        pal.0[1] = [255, 255, 255];
        pal.0[2] = [255, 80, 80];
        let mut y = 2;
        for (i, f) in fonts.0.iter().enumerate() {
            let st = TextStyle::new(i, 1);
            let label = format!("{i}: h{} bpr{} AaBbGg0123 The quick brown fox jumps!", f.height, f.bytes_per_row);
            civrs::gfx::draw_text(&mut s, &fonts, &st, 2, y, &label);
            y += f.line_height() + 4;
            let st2 = TextStyle::new(i, 2);
            let all: String = (f.first..=f.last).map(|c| c as char).collect();
            for line in civrs::gfx::text::wrap(&fonts, i, 316, &all) {
                civrs::gfx::draw_text(&mut s, &fonts, &st2, 2, y, &line);
                y += f.line_height() + 1;
            }
            y += 3;
        }
        s.write_ppm_scaled(&pal, out, zoom)?;
    } else if let Some(rest) = target.strip_prefix("glyphs:") {
        // Every glyph of font N labelled with its hex code (tiny font), for identifying icons.
        let fi: usize = rest.parse().unwrap_or(0);
        let f = fonts.get(fi);
        let mut s = Surface::new(320, 400);
        s.fill(0);
        let mut pal = Palette::black();
        pal.0[1] = [255, 255, 255];
        pal.0[2] = [255, 255, 0];
        let (mut x, mut y) = (2, 2);
        for c in f.first..=f.last {
            civrs::gfx::draw_text(&mut s, &fonts, &TextStyle::new(1, 2), x, y, &format!("{c:02x}"));
            let ch = String::from_utf8_lossy(&[c]).to_string();
            civrs::gfx::draw_text(&mut s, &fonts, &TextStyle::new(fi, 1), x + 14, y, &ch);
            x += 32;
            if x > 300 {
                x = 2;
                y += f.line_height() + 2;
            }
        }
        s.write_ppm_scaled(&pal, out, zoom)?;
    } else if let Some(name) = target.strip_prefix("pal:") {
        let pal = assets.load_pal(name)?;
        let mut s = Surface::new(256, 64);
        for i in 0..256 {
            s.fill_rect(civrs::gfx::Rect::new((i % 16) as i32 * 16, (i / 16) as i32 * 4, 16, 4), i as u8);
        }
        s.write_ppm_scaled(&pal, out, zoom)?;
        for (i, c) in pal.0.iter().enumerate() {
            if i % 8 == 0 {
                print!("\n{i:3}:");
            }
            print!(" ({:3},{:3},{:3})", c[0], c[1], c[2]);
        }
        println!();
    } else if let Some(dir) = target.strip_prefix("all-pics:") {
        std::fs::create_dir_all(dir)?;
        for name in assets.names() {
            if name.ends_with(".PIC") {
                let pic = assets.load_pic(&name)?;
                let pal = pic.palette.unwrap_or_else(Palette::ega);
                let s = Surface::from_pic(&pic);
                let base = name.trim_end_matches(".PIC").to_string();
                s.write_ppm_scaled(&pal, &Path::new(dir).join(format!("{base}.ppm")), zoom)?;
                println!("{name}: {}x{} {:?}", pic.width, pic.height, pic.depth);
            }
        }
    } else if let Some(rest) = target.strip_prefix("map:") {
        // overview of a generated world, 4 px per tile
        let seed: u64 = rest.parse().unwrap_or(1);
        let setup = civrs::game::GameSetup { seed, ..Default::default() };
        let game = civrs::game::Game::new(setup, None);
        dump_overview(&game, out, zoom)?;
    } else if target == "earth" {
        let em = assets.load_earth()?;
        let setup = civrs::game::GameSetup { world: civrs::game::WorldSource::Earth, ..Default::default() };
        let game = civrs::game::Game::new(setup, Some(&em));
        dump_overview(&game, out, zoom)?;
    } else if let Some(rest) = target.strip_prefix("screen:") {
        // Render a screen headlessly: screen:map[:SEED[:TURNS]]
        let parts: Vec<&str> = rest.split(':').collect();
        let seed: u64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
        let turns: u32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        let dir = AssetDir::locate(Some(assets.root()))?;
        let mut ctx = civrs::ui::context::UiContext::new(dir, std::env::temp_dir().join("civrs-headless"))?;
        // previews show the finished screen, not a frame of its dissolve (CIVRS_TRANSITIONS=1
        // keeps the dissolve, for checking it frame by frame with CIVRS_FRAMES)
        ctx.instant = std::env::var("CIVRS_TRANSITIONS").is_err();
        let setup = civrs::game::GameSetup { seed, ..Default::default() };
        let mut game = civrs::game::Game::new(setup, None);
        // previews compare against plain captures: no CIVILIZATION NOTE hints on top
        game.hints_shown = u32::MAX;
        // (mapcity: the map screen after founding that city)
        if !matches!(parts[0], "map" | "mapall" | "founded" | "built") {
            // reports want a city: found one with the first human Settlers
            let slot = game.human_slot;
            if let Some(s) = game.units_of(slot).into_iter().find(|u| game.unit(*u).kind == civrs::rules::unit_ids::SETTLERS) {
                let _ = game.found_city(s);
            }
        }
        for _ in 0..turns {
            let mut r = game.end_turn();
            loop {
                match r.step(&mut game, &mut ctx.ai) {
                    civrs::game::turn::TurnStep::Done => break,
                    civrs::game::turn::TurnStep::Decision(_) => r.resolve(&mut game, civrs::game::DecisionReply::Choice(0)),
                    _ => {}
                }
            }
            let _ = game.take_events();
        }
        // reveal the map for inspection
        if parts.first() == Some(&"mapall") {
            game.map.explore_all(game.human_slot);
        }
        // the city screen needs a city: found one with the human's first settler
        let mut city = None;
        if matches!(parts.first(), Some(&"city" | &"founded" | &"built" | &"diplomat")) {
            let slot = game.human_slot;
            let id = game.units_of(slot).into_iter().find(|id| game.unit(*id).kind == civrs::rules::unit_ids::SETTLERS).ok_or("no settler")?;
            city = Some(game.found_city(id).map_err(|e| format!("{e:?}"))?);
        }
        ctx.game = Some(game);
        let mut dialog: Option<Box<dyn civrs::ui::Screen>> = None;
        let screen: Box<dyn civrs::ui::Screen> = match parts[0] {
            "map" | "mapall" | "mapcity" => Box::new(civrs::ui::mapview::MapView::new()),
            // Civilopedia list: screen:pedia:CATEGORY (0 complete, 1 advances, 2 improvements,
            // 3 units, 4 terrain, 5 misc)
            "pedia" => civrs::ui::civilopedia::category(&ctx, seed as usize),
            // Civilopedia entry: screen:pentry:KII[:PAGE] with K = 0 advance, 1 improvement,
            // 2 wonder, 3 unit, 4 terrain, 5 concept and II the index
            "pentry" => match civrs::ui::civilopedia::entry_by_code(seed as usize) {
                Some(e) => civrs::ui::civilopedia::entry_screen(e, turns as usize),
                None => return Err(format!("unknown civilopedia entry code {seed}").into()),
            },
            "founded" => civrs::ui::cityfounded::city_founded(city.unwrap()),
            // city view after completing an improvement: screen:built:SEED:TURNS[:IMPROVEMENT]
            // (24 + wonder id for a wonder)
            // (CIVRS_FRAMES=70 shows it after the new building has been added)
            "built" => {
                let item: u8 = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(civrs::rules::improvement_ids::GRANARY);
                // items above 24 are wonders (24 + wonder id)
                if item > 24 {
                    let g = ctx.game_mut();
                    g.city_mut(city.unwrap()).add_wonder(item - 24);
                    g.wonders_built[(item - 24) as usize] = city;
                } else {
                    ctx.game_mut().city_mut(city.unwrap()).add_improvement(item);
                }
                civrs::ui::city_view::built_view(&ctx, city.unwrap(), item)
            }
            "city" => match parts.get(3) {
                Some(start) => civrs::ui::cityscreen::city_screen_at(city.unwrap(), start),
                None => civrs::ui::cityscreen::city_screen(city.unwrap()),
            },
            // event screens (DIALOGS track): screen:NAME:ARG
            "discovery" => civrs::ui::discovery::discovery_screen(&ctx, seed as u8),
            "wonder" => civrs::ui::wonderscreen::wonder_screen(&ctx, (seed as u8).clamp(1, 21), "Rome", ctx.game().human_slot),
            "govt" => civrs::ui::govtscreen::govt_screen(&ctx, civrs::rules::Govt::from_u8(seed as u8).unwrap_or(civrs::rules::Govt::Monarchy)),
            "diplomacy" => {
                use civrs::game::diplomacy::{DiplomacyState, Proposal};
                let g = ctx.game_mut();
                let human = g.human_slot;
                g.players[2].in_game = true;
                g.players[2].alive = true;
                g.set_relation(2, human, civrs::game::Relation::Peace);
                let mut st = DiplomacyState::new(2, human, Proposal::Exchange);
                st.rplc1 = "Bronze Working".into();
                st.advance = Some(civrs::rules::advance_ids::BRONZE_WORKING);
                st.human_initiated = true;
                civrs::ui::diplomacy_ui::audience_screen(st)
            }
            // the menu of a Diplomat at a foreign city (the human's first city handed to slot 2):
            // screen:diplomat:SEED:TURNS[:LINE] with LINE the menu line to answer
            "diplomat" => {
                let c = city.unwrap();
                let g = ctx.game_mut();
                g.players[2].in_game = true;
                g.players[2].alive = true;
                g.city_mut(c).owner = 2;
                g.city_mut(c).size = 5;
                g.player_mut(g.human_slot).gold = 900;
                let (x, y) = g.city(c).pos();
                let (dx, dy) = civrs::game::Map::neighbours8(x, y).find(|p| g.map.is_land(p.0, p.1)).ok_or("no land next to the city")?;
                let d = g.add_unit(g.human_slot, civrs::rules::unit_ids::DIPLOMAT, dx, dy);
                let mut view = civrs::ui::mapview::MapView::new();
                let mut t = civrs::ui::unit_actions::diplomat_menu(&mut view, &mut ctx, d, c);
                if let Some(line) = parts.get(3).and_then(|s| s.parse::<usize>().ok()) {
                    t = civrs::ui::unit_actions::diplomat_choice(&mut view, &mut ctx, d, c, line);
                }
                match t {
                    civrs::ui::screen::Transition::Push(s) => civrs::ui::eventpics::over_map(s),
                    _ => return Err("the diplomat's choice shows nothing".into()),
                }
            }
            // crowd marches over the human's first city (10 frames per step of the march)
            "riot" => civrs::ui::eventpics::disorder_screen(&ctx, ctx.game().cities_of(ctx.game().human_slot)[0]),
            "love" => civrs::ui::eventpics::celebration_screen(&ctx, ctx.game().cities_of(ctx.game().human_slot)[0]),
            "invaders" => civrs::ui::eventpics::invaders_screen(&ctx, ctx.game().cities_of(ctx.game().human_slot).first().copied(), vec!["Barbarians land".into(), "near Rome!".into()]),
            "endgame" => civrs::ui::endgame::end_screen(&ctx, match seed { 1 => civrs::game::EndReason::Destroyed, 2 => civrs::game::EndReason::Conquest, 3 => civrs::game::EndReason::Spaceship(1), _ => civrs::game::EndReason::Retired }),
            "arch" => civrs::ui::endgame::arch_screen(&ctx),
            // screen:palace:N[:0:STEP]: the palace after N sample additions; STEP "intro",
            // "section" or "stylePART" opens the addition dialog at that step
            "palace" => {
                let mut pal = civrs::game::palace::Palace::default();
                for k in 0..seed as usize {
                    const ORDER: [usize; 10] = [4, 3, 5, 10, 2, 6, 9, 11, 1, 7];
                    if let Some(part) = ORDER.iter().cycle().skip(k % ORDER.len()).take(ORDER.len()).find_map(|p| pal.pick(*p)) {
                        pal.improve(part, (k % 3) as u8);
                    }
                }
                ctx.game_mut().human_mut().set_palace(&pal);
                match parts.get(3) {
                    Some(&"intro") => civrs::ui::palace::palace_addition(&ctx),
                    Some(step) => civrs::ui::palace::palace_addition_at(&ctx, step),
                    None => civrs::ui::palace::palace_screen(&ctx, Vec::new()),
                }
            }
            "spaceship" => civrs::ui::spaceship_ui::arrival_screen(&ctx, 1),
            // the human's ship (a sample with every kind of part, one module unattached):
            // screen:ship:SEED:TURNS[:STEP] with STEP "confirm" (the launch question), "nation"
            // (the viewer's "Which nation?"), "launched", "notice" (ship_launch's text) or
            // "part" (the "Type?" question over the ship)
            "ship" => {
                use civrs::ui::spaceship_ui as ss;
                let g = ctx.game_mut();
                let h = g.human_slot;
                g.apollo_built = true;
                g.set_turn(370);
                for _ in 0..12 {
                    g.place_spaceship_part(h, 0, 0);
                }
                for c in [0, 1, 0, 1] {
                    g.place_spaceship_part(h, 1, c);
                }
                for c in [0, 1, 2, 0] {
                    g.place_spaceship_part(h, 2, c);
                }
                let step = parts.get(3).copied().unwrap_or("");
                if matches!(step, "launched" | "notice") {
                    g.launch_spaceship(h).map_err(|e| format!("{e:?}"))?;
                }
                let _ = g.take_events();
                match step {
                    "confirm" => {
                        dialog = Some(Box::new(ss::launch_confirm(ctx.game(), h)));
                        ss::viewer_ship(&ctx, h)
                    }
                    "nation" => {
                        dialog = Some(Box::new(ss::nation_menu(ctx.game())));
                        Box::new(civrs::ui::mapview::MapView::new())
                    }
                    "notice" => ss::launch_screens(&ctx, h).0,
                    "part" => {
                        let items = vec!["Habitation".into(), "Life Support".into(), "Solar Panel".into()];
                        dialog = Some(Box::new(civrs::ui::widgets::MenuDialog::new(vec!["Type?".into()], items).with_pos(100, 80).no_cancel()));
                        ss::ship_view(&ctx, h)
                    }
                    _ => ss::viewer_ship(&ctx, h),
                }
            }
            "news" => civrs::ui::eventpics::over_map(civrs::ui::eventpics::travelers_report(vec!["Babylonian civilization".into(), "destroyed by Romans!".into()])),
            "rednote" => civrs::ui::eventpics::over_map(civrs::ui::dialogs::error_screen(&ctx, &civrs::game::RuleError::Settlers).ok_or("no ERROR.TXT section")?),
            "nuke" => civrs::ui::eventpics::over_map(civrs::ui::eventpics::nuke_animation(200, 104)),
            "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12" => {
                let n: u8 = parts[0][1..].parse().unwrap_or(1);
                civrs::ui::reports::advisor(&ctx, n)
            }
            "fame" => civrs::ui::halloffame::hall_of_fame(),
            "tax" | "lux" | "options" | "findcity" | "save" | "load" | "loadtitle" => {
                dialog = Some(match parts[0] {
                    "tax" => civrs::ui::reports::rate_dialog(&ctx, false),
                    "lux" => civrs::ui::reports::rate_dialog(&ctx, true),
                    "options" => civrs::ui::reports::options_dialog(&ctx),
                    "findcity" => civrs::ui::reports::find_city_dialog(&ctx),
                    "save" => civrs::ui::reports::save_dialog(&ctx),
                    "load" => civrs::ui::reports::load_dialog(&ctx),
                    _ => civrs::ui::reports::load_game_screen(),
                });
                Box::new(civrs::ui::mapview::MapView::new())
            }
            other => return Err(format!("unknown screen {other}").into()),
        };
        let mut app = civrs::app::App::new(ctx, screen);
        if let Some(d) = dialog {
            app.screens.push(d, &mut app.ctx);
        }
        let mut backend = civrs::platform::headless::HeadlessBackend::new();
        let frames: u32 = std::env::var("CIVRS_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
        for _ in 0..frames {
            app.frame(&mut backend);
        }
        let pal = app.palette();
        app.fb.write_ppm_scaled(&pal, out, zoom)?;
    } else {
        return Err(format!("unknown dump target {target}").into());
    }
    Ok(())
}

/// Terrain colours for the overview map (palette index -> rgb set below).
fn dump_overview(game: &civrs::game::Game, out: &Path, zoom: usize) -> Result<(), Box<dyn std::error::Error>> {
    use civrs::rules::Terrain;
    let mut pal = Palette::black();
    let colours: [(Terrain, [u8; 3]); 12] = [
        (Terrain::Desert, [230, 210, 120]), (Terrain::Plains, [190, 170, 80]), (Terrain::Grassland, [90, 180, 60]),
        (Terrain::Forest, [30, 110, 30]), (Terrain::Hills, [150, 120, 60]), (Terrain::Mountains, [140, 140, 140]),
        (Terrain::Tundra, [170, 170, 150]), (Terrain::Arctic, [240, 240, 240]), (Terrain::Swamp, [70, 110, 90]),
        (Terrain::Jungle, [40, 150, 90]), (Terrain::Ocean, [40, 60, 160]), (Terrain::River, [80, 120, 220]),
    ];
    for (t, c) in colours {
        pal.0[t as usize + 1] = c;
    }
    pal.0[20] = [255, 0, 0]; // city / start
    pal.0[21] = [255, 255, 0]; // hut
    pal.0[22] = [255, 255, 255]; // special
    let w = civrs::rules::MAP_W as i32;
    let h = civrs::rules::MAP_H as i32;
    let mut s = Surface::new(w * 4, h * 4);
    for y in 0..h {
        for x in 0..w {
            let t = game.map.tile(x, y);
            s.fill_rect(civrs::gfx::Rect::new(x * 4, y * 4, 4, 4), t.terrain as u8 + 1);
            if t.special {
                s.put(x * 4 + 1, y * 4 + 1, 22);
            }
            if t.has(civrs::game::tile_flags::HUT) {
                s.put(x * 4 + 2, y * 4 + 2, 21);
            }
        }
    }
    for u in game.all_units() {
        s.fill_rect(civrs::gfx::Rect::new(u.x * 4 + 1, u.y * 4 + 1, 2, 2), 20);
    }
    let land = game.map.count_land();
    println!("land tiles: {land} ({}%), units: {}", land * 100 / (80 * 50), game.all_units().count());
    s.write_ppm_scaled(&pal, out, zoom)?;
    Ok(())
}

/// Render one sound ID through the driver and the OPL2 emulator: a 48 kHz stereo WAV, or
/// (`regs`) a text log `subtick reg value` of the register writes, one 300 Hz sub-tick per
/// line group, for comparison with the reference simulator.
fn dump_sound(assets: &AssetDir, spec: &str, out: &Path, regs: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut parts = spec.split(':');
    let id: u8 = parts.next().unwrap_or("3").parse()?;
    let level: u8 = parts.next().map(|s| s.parse()).transpose()?.unwrap_or(3);
    let bytes = assets.read("ASOUND.CVL")?;
    let im = civrs::audio::cvl::Image::parse(&bytes)?;
    let rate = civrs::audio::SAMPLE_RATE;
    let mut engine = civrs::audio::Engine::new(civrs::audio::driver::Driver::new(im), rate);
    engine.play(id, level);
    if regs {
        let ticks: u32 = parts.next().map(|s| s.parse()).transpose()?.unwrap_or(600);
        let mut text = String::new();
        for sub in 0..ticks * 5 {
            for (r, v) in engine.step_sub_tick() {
                text.push_str(&format!("{sub} {r:02x} {v:02x}\n"));
            }
        }
        std::fs::write(out, text)?;
        println!("{}: {id} level {level}, {ticks} ticks", out.display());
        return Ok(());
    }
    let seconds: f64 = parts.next().map(|s| s.parse()).transpose()?.unwrap_or(30.0);
    let mut pcm: Vec<i16> = Vec::new();
    let chunk = (rate / 50) as usize * 2;
    let mut buf = vec![0i16; chunk];
    let mut silent = 0usize;
    while (pcm.len() / 2) < (seconds * rate as f64) as usize {
        engine.render(&mut buf);
        pcm.extend_from_slice(&buf);
        silent = if engine.is_active() { 0 } else { silent + 1 };
        if silent > 75 {
            break; // 1.5 s with nothing left to play
        }
    }
    let data_len = (pcm.len() * 2) as u32;
    let mut w: Vec<u8> = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&2u16.to_le_bytes()); // stereo
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in &pcm {
        w.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(out, w)?;
    println!("{}: sound {id} level {level}, {:.1} s", out.display(), pcm.len() as f64 / 2.0 / rate as f64);
    Ok(())
}

fn main() {
    let cli = parse_cli();
    let assets = match AssetDir::locate(cli.assets.as_deref()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    if let Some(target) = &cli.dump {
        let out = cli.out.clone().unwrap_or_else(|| PathBuf::from("dump.ppm"));
        if let Err(e) = dump(&assets, target, &out, cli.zoom) {
            eprintln!("dump failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    let save_dir = civrs::save::default_save_dir();
    let mut ctx = match civrs::ui::context::UiContext::new(assets, save_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error loading assets: {e}");
            std::process::exit(1);
        }
    };
    let first: Box<dyn civrs::ui::Screen> = if cli.skip_intro {
        let setup = civrs::game::GameSetup { seed: cli.seed.unwrap_or(7), ..Default::default() };
        ctx.game = Some(civrs::game::Game::new(setup, None));
        Box::new(civrs::ui::mapview::MapView::new())
    } else {
        Box::new(civrs::ui::title::TitleScreen::new())
    };
    let opts = civrs::platform::PlatformOptions { scale: cli.scale, fullscreen: cli.fullscreen, title: "Civilization".into() };
    let mut backend = match civrs::platform::sdl::SdlBackend::init(&opts) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("could not initialise SDL: {e}");
            std::process::exit(1);
        }
    };
    let mut app = civrs::app::App::new(ctx, first);
    if let Some(engine) = app.ctx.audio.engine() {
        if let Err(e) = backend.start_audio(engine) {
            eprintln!("sound: {e}; running silent");
        }
    }
    app.run(&mut backend);
}
