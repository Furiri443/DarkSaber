//! Các il2cpp hook đọc cờ trong `state` để can thiệp chỉ số / quyền năng.
//!
//! Bài học từ RE (IDA): nhiều getter KHÔNG được game gọi trong hot-path (game đọc
//! field trực tiếp), nên hook getter không ăn. Cách chắc ăn = GHI THẲNG FIELD mỗi
//! frame trong PlayerController.Update (ta có `this`, chạy trên thread VM).
//!
//! Field offset (xác minh từ IDA, GameAssembly.dll base 0x180000000):
//!   PlayerController.moveSpeed   @ +184 (0xB8)  float   (get_MoveSpeed)
//!   PlayerController.bulletSpeed @ +216 (0xD8)  float   (get_BulletSpeed)
//!   PlayerController.fireRate    @ +220 (0xDC)  float   (get_FireRate)
//!   PlayerController.gold        @ +52  (0x34)  int     (GoldDropItem.OnCollect cộng vào đây)
//!   PlayerController.curHp       @ +56  (0x38)  int     (TakeDamage trừ vào đây)
//!   GetMaxHp = giá trị tính toán → hook return để nâng max.
//!
//! ABI: instance method = fn(this, <args...>, MethodInfo* method).

use core::ffi::c_void;
use core::sync::atomic::Ordering::Relaxed;

use retour::static_detour;

use crate::il2cpp::{Il2Cpp, Image};
use crate::log_line;
use crate::state::{self, Callable, Powers};

const NS: &str = "Controller.PlayerController";
const PLAYER: &str = "PlayerController";
const SHIELD: &str = "PlayerShield";
const ENEMY: &str = "EnemyManager";
const NPC_NS: &str = "Controller.NPCController";
const BUFF_UI: &str = "BuffUIItem";

// Field offset trong object PlayerController.
const OFF_MOVE_SPEED: usize = 184;
const OFF_BULLET_SPEED: usize = 216;
const OFF_FIRE_RATE: usize = 220;
const OFF_GOLD: usize = 52;
const OFF_CUR_HP: usize = 56;
const OFF_MAX_HP: usize = 60;
const OFF_HP_FLAT_BONUS: usize = 76;
const OFF_HP_PERCENT_BONUS: usize = 80;
const OFF_ATK_FLAT_BONUS: usize = 84;
const OFF_ATK_PERCENT_BONUS: usize = 88;
const OFF_HEAL_FLAG: usize = 92;
const OFF_SHIELD_UNLOCKED: usize = 96;
const OFF_EXTRA_BULLETS: usize = 100;
const OFF_PLAYER_SHIELD_HP: usize = 104;
const OFF_NEXT_FIRE_TIME: usize = 304;
const OFF_SHOOT_TIMER: usize = 308;
const OFF_SHIELD_ACTIVE: usize = 56;

static_detour! {
    static GetMaxHp: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
    static TakeDamage: unsafe extern "C" fn(*mut c_void, i32, *mut c_void);
    static PlayerUpdate: unsafe extern "C" fn(*mut c_void, *mut c_void);
    static ShieldStart: unsafe extern "C" fn(*mut c_void, *mut c_void);
    static ShieldUpdate: unsafe extern "C" fn(*mut c_void, *mut c_void);
    static EnemyTakeDamage: unsafe extern "C" fn(*mut c_void, i32, *mut c_void);
    static BuffSetup: unsafe extern "C" fn(*mut c_void, i32, i32, i32, i32, i32, *mut c_void, *mut c_void);
}

type Fn0 = unsafe extern "C" fn(*mut c_void, *mut c_void); // (this, method)
type Fn1i = unsafe extern "C" fn(*mut c_void, i32, *mut c_void); // (this, int, method)

#[derive(Default, Clone, Copy)]
struct StatSnapshot {
    player: usize,
    move_valid: bool,
    bullet_valid: bool,
    fire_valid: bool,
    move_orig: f32,
    bullet_orig: f32,
    fire_orig: f32,
    last_speed_on: bool,
    last_bullet_on: bool,
    last_fire_on: bool,
}

static mut STAT_SNAPSHOT: StatSnapshot = StatSnapshot {
    player: 0,
    move_valid: false,
    bullet_valid: false,
    fire_valid: false,
    move_orig: 0.0,
    bullet_orig: 0.0,
    fire_orig: 0.0,
    last_speed_on: false,
    last_bullet_on: false,
    last_fire_on: false,
};

unsafe fn call0(c: &Callable, this: usize) {
    let f: Fn0 = core::mem::transmute(c.code);
    f(this as *mut c_void, c.method as *mut c_void);
}
unsafe fn call1i(c: &Callable, this: usize, arg: i32) {
    let f: Fn1i = core::mem::transmute(c.code);
    f(this as *mut c_void, arg, c.method as *mut c_void);
}

/// Cài toàn bộ hook + resolve các method power. Gọi một lần.
pub unsafe fn install(api: &Il2Cpp, image: Image) {
    // GetMaxHp: nâng max hp khi bật override.
    if let Some(addr) = api.method_address(image, NS, PLAYER, "GetMaxHp", 0) {
        let t: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 = core::mem::transmute(addr);
        let _ = GetMaxHp
            .initialize(t, on_max_hp)
            .and_then(|_| GetMaxHp.enable());
    }
    // TakeDamage: god mode + bắt this.
    if let Some(addr) = api.method_address(image, NS, PLAYER, "TakeDamage", 1) {
        let t: unsafe extern "C" fn(*mut c_void, i32, *mut c_void) = core::mem::transmute(addr);
        let _ = TakeDamage
            .initialize(t, on_take_damage)
            .and_then(|_| TakeDamage.enable());
    }
    // PlayerController.Update: ghi field chỉ số + pump power mỗi frame.
    if let Some(addr) = api.method_address(image, NS, PLAYER, "Update", 0) {
        let t: unsafe extern "C" fn(*mut c_void, *mut c_void) = core::mem::transmute(addr);
        let _ = PlayerUpdate
            .initialize(t, on_player_update)
            .and_then(|_| PlayerUpdate.enable());
    }
    // PlayerShield.Start: bắt instance shield (Start chạy 1 lần lúc spawn).
    if let Some(addr) = api.method_address(image, NS, SHIELD, "Start", 0) {
        let t: unsafe extern "C" fn(*mut c_void, *mut c_void) = core::mem::transmute(addr);
        let _ = ShieldStart
            .initialize(t, on_shield_start)
            .and_then(|_| ShieldStart.enable());
    }
    // PlayerShield.Update: bắt instance liên tục; nhiều scene tắt/bật component làm Start chưa đủ.
    if let Some(addr) = api.method_address(image, NS, SHIELD, "Update", 0) {
        let t: unsafe extern "C" fn(*mut c_void, *mut c_void) = core::mem::transmute(addr);
        let _ = ShieldUpdate
            .initialize(t, on_shield_update)
            .and_then(|_| ShieldUpdate.enable());
    }
    // EnemyManager.TakeDamage: One-Hit Kill.
    if let Some(addr) = api.method_address(image, "", ENEMY, "TakeDamage", 1) {
        let t: unsafe extern "C" fn(*mut c_void, i32, *mut c_void) = core::mem::transmute(addr);
        let _ = EnemyTakeDamage
            .initialize(t, on_enemy_take_damage)
            .and_then(|_| EnemyTakeDamage.enable());
    }
    if let Some(addr) = api.method_address(image, NPC_NS, BUFF_UI, "Setup", 6) {
        let t: unsafe extern "C" fn(
            *mut c_void,
            i32,
            i32,
            i32,
            i32,
            i32,
            *mut c_void,
            *mut c_void,
        ) = core::mem::transmute(addr);
        let _ = BuffSetup
            .initialize(t, on_buff_setup)
            .and_then(|_| BuffSetup.enable());
    }

    // Resolve các power để gọi trực tiếp.
    let mk = |m: &str, argc: i32| -> Option<Callable> {
        api.resolve_method(image, NS, PLAYER, m, argc)
            .map(|(code, method)| Callable { code, method })
    };
    let mk_shield = |m: &str, argc: i32| -> Option<Callable> {
        api.resolve_method(image, NS, SHIELD, m, argc)
            .map(|(code, method)| Callable { code, method })
    };
    let powers = Powers {
        revive: mk("RevivePlayer", 0),
        add_exp: mk("AddExp", 1),
        add_item: mk("AddItem", 1),
        get_max_hp: mk("GetMaxHp", 0),
        save_game: mk("SaveGame", 0),
        shield_on: mk_shield("ActivateShield", 0),
        shield_off: mk_shield("DeactivateShield", 0),
    };

    let report = format!(
        "Hooks: MaxHp={} TakeDamage={} Update={} ShieldStart={} ShieldUpdate={} EnemyDamage={} BuffSetup={}\n\
         Stats: snapshot/restore Speed/Bullet/Cooldown + HP/Shield lock + captured buffs\n\
         Powers: Revive={} AddExp={} AddItem={} GetMaxHp={} SaveGame={} Shield={}",
        ok(GetMaxHp.is_enabled()),
        ok(TakeDamage.is_enabled()),
        ok(PlayerUpdate.is_enabled()),
        ok(ShieldStart.is_enabled()),
        ok(ShieldUpdate.is_enabled()),
        ok(EnemyTakeDamage.is_enabled()),
        ok(BuffSetup.is_enabled()),
        ok(powers.revive.is_some()),
        ok(powers.add_exp.is_some()),
        ok(powers.add_item.is_some()),
        ok(powers.get_max_hp.is_some()),
        ok(powers.save_game.is_some()),
        ok(powers.shield_on.is_some() && powers.shield_off.is_some()),
    );
    log_line(&format!("[hook] {}", report.replace('\n', " | ")));
    let _ = state::HOOK_REPORT.set(report);
    let _ = state::POWERS.set(powers);
}

fn ok(b: bool) -> &'static str {
    if b {
        "OK"
    } else {
        "x"
    }
}

// ───────────────────────── Detour callbacks ─────────────────────────

fn on_max_hp(this: *mut c_void, mi: *mut c_void) -> i32 {
    let orig = unsafe { GetMaxHp.call(this, mi) };
    if state::MAXHP_OVERRIDE.load(Relaxed) {
        state::MAXHP_VALUE.load(Relaxed)
    } else {
        orig
    }
}

fn on_take_damage(this: *mut c_void, amount: i32, mi: *mut c_void) {
    state::PLAYER_INSTANCE.store(this as usize, Relaxed);
    if state::GOD_MODE.load(Relaxed) {
        return; // nuốt sát thương
    }
    unsafe { TakeDamage.call(this, amount, mi) }
}

unsafe fn f32_at(base: usize, off: usize) -> *mut f32 {
    (base + off) as *mut f32
}

unsafe fn i32_at(base: usize, off: usize) -> *mut i32 {
    (base + off) as *mut i32
}

unsafe fn u8_at(base: usize, off: usize) -> *mut u8 {
    (base + off) as *mut u8
}

unsafe fn save_player_if_available(pw: &Powers, player: usize) {
    if let Some(c) = &pw.save_game {
        call0(c, player);
    }
}

unsafe fn get_max_hp_if_available(pw: &Powers, player: usize) -> Option<i32> {
    let c = pw.get_max_hp.as_ref()?;
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 = core::mem::transmute(c.code);
    Some(f(player as *mut c_void, c.method as *mut c_void))
}

unsafe fn apply_captured_buff(pw: &Powers, player: usize, buff: &state::BuffOption) {
    match buff.stat_type {
        0 => {
            let off = if buff.is_percent {
                OFF_HP_PERCENT_BONUS
            } else {
                OFF_HP_FLAT_BONUS
            };
            *i32_at(player, off) = (*i32_at(player, off)).saturating_add(buff.value);
            if let Some(max_hp) = get_max_hp_if_available(pw, player) {
                *i32_at(player, OFF_MAX_HP) = max_hp;
            }
        }
        1 => {
            let off = if buff.is_percent {
                OFF_ATK_PERCENT_BONUS
            } else {
                OFF_ATK_FLAT_BONUS
            };
            *i32_at(player, off) = (*i32_at(player, off)).saturating_add(buff.value);
        }
        2 => {
            *f32_at(player, OFF_MOVE_SPEED) += buff.value as f32;
        }
        3 => {
            let delta = (buff.value.max(1)) as f32;
            *f32_at(player, OFF_FIRE_RATE) = (*f32_at(player, OFF_FIRE_RATE) - delta).max(0.01);
            *f32_at(player, OFF_NEXT_FIRE_TIME) = 0.0;
            *f32_at(player, OFF_SHOOT_TIMER) = 0.0;
        }
        4 => {
            *f32_at(player, OFF_BULLET_SPEED) += buff.value as f32;
        }
        5 => {
            *i32_at(player, OFF_CUR_HP) = *i32_at(player, OFF_MAX_HP);
            *i32_at(player, OFF_HEAL_FLAG) = 1;
        }
        6 => {
            *i32_at(player, OFF_SHIELD_UNLOCKED) = 1;
            *i32_at(player, OFF_PLAYER_SHIELD_HP) = (*i32_at(player, OFF_PLAYER_SHIELD_HP)).max(1);
            let shield = state::SHIELD_INSTANCE.load(Relaxed);
            if shield != 0 {
                *u8_at(shield, OFF_SHIELD_ACTIVE) = 1;
            }
        }
        7 => {
            *i32_at(player, OFF_EXTRA_BULLETS) =
                (*i32_at(player, OFF_EXTRA_BULLETS)).saturating_add(buff.value);
        }
        _ => {}
    }
}

/// Ghi field theo cờ (base = PlayerController object), có snapshot/restore.
unsafe fn apply_stats(base: usize) {
    let s = &mut *core::ptr::addr_of_mut!(STAT_SNAPSHOT);
    if s.player != base {
        *s = StatSnapshot {
            player: base,
            ..StatSnapshot::default()
        };
    }

    let speed_on = state::SPEED_HACK.load(Relaxed);
    let bullet_on = state::FAST_BULLETS.load(Relaxed);
    let fire_on = state::RAPID_FIRE.load(Relaxed);

    if speed_on {
        if !s.move_valid {
            s.move_orig = *f32_at(base, OFF_MOVE_SPEED);
            s.move_valid = true;
        }
        *f32_at(base, OFF_MOVE_SPEED) = state::speed_value();
    } else if s.last_speed_on && s.move_valid {
        *f32_at(base, OFF_MOVE_SPEED) = s.move_orig;
        s.move_valid = false;
    }

    if bullet_on {
        if !s.bullet_valid {
            s.bullet_orig = *f32_at(base, OFF_BULLET_SPEED);
            s.bullet_valid = true;
        }
        *f32_at(base, OFF_BULLET_SPEED) = state::bullet_speed_value();
    } else if s.last_bullet_on && s.bullet_valid {
        *f32_at(base, OFF_BULLET_SPEED) = s.bullet_orig;
        s.bullet_valid = false;
    }

    if fire_on {
        if !s.fire_valid {
            s.fire_orig = *f32_at(base, OFF_FIRE_RATE);
            s.fire_valid = true;
        }
        let cooldown = state::fire_rate_value().clamp(0.01, 5.0);
        *f32_at(base, OFF_FIRE_RATE) = cooldown;
        // Tránh chờ timer cũ khi vừa bật rapid; không gọi Shoot trực tiếp để khỏi lặp.
        *f32_at(base, OFF_NEXT_FIRE_TIME) = 0.0;
        *f32_at(base, OFF_SHOOT_TIMER) = 0.0;
    } else if s.last_fire_on && s.fire_valid {
        *f32_at(base, OFF_FIRE_RATE) = s.fire_orig;
        s.fire_valid = false;
    }

    s.last_speed_on = speed_on;
    s.last_bullet_on = bullet_on;
    s.last_fire_on = fire_on;
}

unsafe fn apply_shield_lock(player: usize) {
    if state::SHIELD_LOCK.load(Relaxed) {
        *i32_at(player, OFF_PLAYER_SHIELD_HP) = state::SHIELD_VALUE.load(Relaxed).max(1);

        let shield = state::SHIELD_INSTANCE.load(Relaxed);
        if shield != 0 {
            *u8_at(shield, OFF_SHIELD_ACTIVE) = 1;
        }
    }
}

fn on_player_update(this: *mut c_void, mi: *mut c_void) {
    let p = this as usize;
    state::PLAYER_INSTANCE.store(p, Relaxed);

    // Ghi chỉ số TRƯỚC khi Update gốc chạy (để HandleMovement/Shoot frame này dùng).
    unsafe {
        apply_stats(p);
        apply_shield_lock(p);
    };

    // Pump power one-shot (đang ở thread game đã attach VM).
    if let Some(pw) = state::POWERS.get() {
        if state::REQ_REVIVE.swap(false, Relaxed) {
            if let Some(c) = &pw.revive {
                unsafe { call0(c, p) };
            }
        }
        if state::REQ_ADD_EXP.swap(false, Relaxed) {
            if let Some(c) = &pw.add_exp {
                let amount = state::EXP_AMOUNT.load(Relaxed).max(0);
                unsafe {
                    call1i(c, p, amount);
                    save_player_if_available(pw, p);
                };
                log_line(&format!("[power] AddExp +{}", amount));
            }
        }
        if state::REQ_ADD_GOLD.swap(false, Relaxed) {
            let amount = state::GOLD_AMOUNT.load(Relaxed).max(0);
            if amount > 0 {
                unsafe {
                    let gold = i32_at(p, OFF_GOLD);
                    *gold = (*gold).saturating_add(amount);
                    save_player_if_available(pw, p);
                };
                log_line(&format!("[power] AddGold +{}", amount));
            }
        }
        if state::REQ_ADD_ITEM.swap(false, Relaxed) {
            if let Some(c) = &pw.add_item {
                let id = state::ITEM_ID.load(Relaxed);
                unsafe {
                    call1i(c, p, id);
                    save_player_if_available(pw, p);
                };
                log_line(&format!("[power] AddItem raw id={}", id));
            }
        }
        if state::REQ_ADD_ITEM_PRESET.swap(false, Relaxed) {
            if let Some(c) = &pw.add_item {
                let item = state::item_preset(state::ITEM_PRESET_INDEX.load(Relaxed));
                unsafe {
                    call1i(c, p, item.id);
                    save_player_if_available(pw, p);
                };
                log_line(&format!("[power] AddItem {} id={}", item.name, item.id));
            }
        }
        if state::REQ_ADD_KNOWN_ITEMS.swap(false, Relaxed) {
            if let Some(c) = &pw.add_item {
                for item in state::ITEM_PRESETS {
                    unsafe { call1i(c, p, item.id) };
                }
                unsafe { save_player_if_available(pw, p) };
                log_line("[power] Add known items requested");
            }
        }
        if state::REQ_APPLY_SELECTED_BUFFS.swap(false, Relaxed) {
            let mask = state::BUFF_SELECTED_MASK.load(Relaxed);
            if mask != 0 {
                let buffs = state::known_buffs_snapshot();
                let mut applied = 0usize;
                for (idx, buff) in buffs.iter().enumerate() {
                    if (mask & (1usize << idx)) == 0 {
                        continue;
                    }
                    unsafe { apply_captured_buff(pw, p, buff) };
                    applied += 1;
                    log_line(&format!(
                        "[power] Buff {} value={} cost={}",
                        state::buff_kind_name(buff.stat_type, buff.is_percent),
                        buff.value,
                        buff.cost
                    ));
                }
                if applied != 0 {
                    unsafe { save_player_if_available(pw, p) };
                    log_line(&format!("[power] Applied {} selected buff(s)", applied));
                }
            }
        }
        // Shield: gọi trên instance đã bắt ở Shield.Start.
        let s = state::SHIELD_INSTANCE.load(Relaxed);
        if s != 0 {
            if state::REQ_SHIELD_ON.swap(false, Relaxed) {
                if let Some(c) = &pw.shield_on {
                    unsafe { call0(c, s) };
                }
            }
            if state::REQ_SHIELD_OFF.swap(false, Relaxed) {
                if let Some(c) = &pw.shield_off {
                    unsafe { call0(c, s) };
                }
            }
        }
    }

    unsafe { PlayerUpdate.call(this, mi) };

    // Khoá HP/Shield SAU Update (ghi đè mọi thay đổi trong frame).
    unsafe {
        if state::MAXHP_OVERRIDE.load(Relaxed) {
            *i32_at(p, OFF_CUR_HP) = state::MAXHP_VALUE.load(Relaxed);
        }
        apply_shield_lock(p);
    }
}

fn on_shield_start(this: *mut c_void, mi: *mut c_void) {
    state::SHIELD_INSTANCE.store(this as usize, Relaxed);
    unsafe { ShieldStart.call(this, mi) };
}

fn on_shield_update(this: *mut c_void, mi: *mut c_void) {
    state::SHIELD_INSTANCE.store(this as usize, Relaxed);
    if state::SHIELD_LOCK.load(Relaxed) {
        unsafe { *u8_at(this as usize, OFF_SHIELD_ACTIVE) = 1 };
    }
    unsafe { ShieldUpdate.call(this, mi) };
    if state::SHIELD_LOCK.load(Relaxed) {
        unsafe { *u8_at(this as usize, OFF_SHIELD_ACTIVE) = 1 };
    }
}

fn on_enemy_take_damage(this: *mut c_void, amount: i32, mi: *mut c_void) {
    let amount = if state::ONE_HIT_KILL.load(Relaxed) {
        999_999
    } else {
        amount
    };
    unsafe { EnemyTakeDamage.call(this, amount, mi) };
}

fn on_buff_setup(
    this: *mut c_void,
    slot: i32,
    cost: i32,
    stat_type: i32,
    value: i32,
    is_percent: i32,
    _npc: *mut c_void,
    mi: *mut c_void,
) {
    if slot <= 0 {
        state::clear_captured_buffs();
    }
    state::upsert_captured_buff(state::BuffOption {
        slot,
        cost,
        stat_type,
        value,
        is_percent: is_percent != 0,
    });
    log_line(&format!(
        "[buff] slot={} {} value={} cost={}",
        slot,
        state::buff_kind_name(stat_type, is_percent != 0),
        value,
        cost
    ));
    unsafe { BuffSetup.call(this, slot, cost, stat_type, value, is_percent, _npc, mi) };
}

/// Tìm image game theo tên ("Assembly-CSharp").
pub unsafe fn find_game_image(api: &Il2Cpp, name: &str) -> Image {
    let domain = (api.domain_get)();
    let mut count: usize = 0;
    let assemblies = (api.domain_get_assemblies)(domain, &mut count);
    for i in 0..count {
        let assembly = *assemblies.add(i);
        if assembly.is_null() {
            continue;
        }
        let image = (api.assembly_get_image)(assembly);
        if image.is_null() {
            continue;
        }
        let img_name = crate::il2cpp::cstr_to_string((api.image_get_name)(image));
        if img_name.starts_with(name) {
            return image;
        }
    }
    core::ptr::null_mut()
}
