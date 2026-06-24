//! Trạng thái chia sẻ giữa overlay (thread render) và các il2cpp hook (thread game).
//!
//! Tất cả dùng atomic để truy cập không khoá. Overlay GHI, hook ĐỌC; với power
//! one-shot thì overlay set cờ request, hook trên thread game tiêu thụ (vì gọi
//! managed method phải ở thread đã attach VM).

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

// ───────────────────────── UI ─────────────────────────
/// Menu overlay đang hiện hay không (bật/tắt bằng phím `\`).
pub static MENU_VISIBLE: AtomicBool = AtomicBool::new(false);

// ──────────────────── Toggle chỉ số ────────────────────
pub static GOD_MODE: AtomicBool = AtomicBool::new(false);

pub static SPEED_HACK: AtomicBool = AtomicBool::new(false);
static SPEED_VALUE: AtomicU32f = AtomicU32f::new(8.0);

pub static RAPID_FIRE: AtomicBool = AtomicBool::new(false);
/// Đây là cooldown giữa 2 viên đạn, không phải "rate". Nhỏ hơn = bắn nhanh hơn.
static FIRE_RATE_VALUE: AtomicU32f = AtomicU32f::new(0.05);

pub static FAST_BULLETS: AtomicBool = AtomicBool::new(false);
static BULLET_SPEED_VALUE: AtomicU32f = AtomicU32f::new(30.0);

pub static MAXHP_OVERRIDE: AtomicBool = AtomicBool::new(false);
pub static MAXHP_VALUE: AtomicI32 = AtomicI32::new(9999);

pub static SHIELD_LOCK: AtomicBool = AtomicBool::new(false);
pub static SHIELD_VALUE: AtomicI32 = AtomicI32::new(9999);

pub static ONE_HIT_KILL: AtomicBool = AtomicBool::new(false);

// ──────────────── Instance bắt được từ hook ────────────────
/// Con trỏ `this` của PlayerController, bắt trong Update/TakeDamage.
pub static PLAYER_INSTANCE: AtomicUsize = AtomicUsize::new(0);
/// Con trỏ `this` của PlayerShield, bắt trong Shield.Start.
pub static SHIELD_INSTANCE: AtomicUsize = AtomicUsize::new(0);

// ──────────────── Power one-shot (request) ────────────────
pub static REQ_REVIVE: AtomicBool = AtomicBool::new(false);
pub static REQ_ADD_EXP: AtomicBool = AtomicBool::new(false);
pub static EXP_AMOUNT: AtomicI32 = AtomicI32::new(1000);
pub static REQ_ADD_GOLD: AtomicBool = AtomicBool::new(false);
pub static GOLD_AMOUNT: AtomicI32 = AtomicI32::new(1000);
pub static REQ_ADD_ITEM: AtomicBool = AtomicBool::new(false);
pub static ITEM_ID: AtomicI32 = AtomicI32::new(1);
pub static REQ_ADD_ITEM_PRESET: AtomicBool = AtomicBool::new(false);
pub static ITEM_PRESET_INDEX: AtomicI32 = AtomicI32::new(0);
pub static REQ_ADD_KNOWN_ITEMS: AtomicBool = AtomicBool::new(false);
pub static REQ_APPLY_SELECTED_BUFFS: AtomicBool = AtomicBool::new(false);
pub static BUFF_SELECTED_MASK: AtomicUsize = AtomicUsize::new(0);
pub static REQ_SHIELD_ON: AtomicBool = AtomicBool::new(false);
pub static REQ_SHIELD_OFF: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub struct ItemPreset {
    pub id: i32,
    pub name: &'static str,
}

/// Các tên item tìm được từ dump/asset string. ID vẫn cần kiểm chứng runtime theo
/// build cụ thể, nhưng tốt hơn quét mù 1..N vì inventory game chỉ có 3 slot.
pub const ITEM_PRESETS: &[ItemPreset] = &[
    ItemPreset {
        id: 1,
        name: "GateKey",
    },
    ItemPreset {
        id: 2,
        name: "GateKey1",
    },
    ItemPreset {
        id: 3,
        name: "GateKey2",
    },
    ItemPreset {
        id: 4,
        name: "GateKey3",
    },
    ItemPreset {
        id: 5,
        name: "RedBulletGift",
    },
    ItemPreset {
        id: 6,
        name: "GreenBulletGift",
    },
    ItemPreset {
        id: 7,
        name: "YellowBulletGift",
    },
    ItemPreset {
        id: 8,
        name: "Key1",
    },
    ItemPreset {
        id: 9,
        name: "Key3",
    },
];

pub fn item_preset(index: i32) -> ItemPreset {
    let idx = index.max(0) as usize % ITEM_PRESETS.len();
    ITEM_PRESETS[idx]
}

pub fn item_preset_count() -> i32 {
    ITEM_PRESETS.len() as i32
}

#[derive(Clone, Copy, Default)]
pub struct BuffOption {
    pub slot: i32,
    pub cost: i32,
    pub stat_type: i32,
    pub value: i32,
    pub is_percent: bool,
}

pub const KNOWN_BUFFS: &[BuffOption] = &[
    BuffOption {
        slot: 0,
        cost: 0,
        stat_type: 0,
        value: 25,
        is_percent: false,
    },
    BuffOption {
        slot: 1,
        cost: 0,
        stat_type: 0,
        value: 10,
        is_percent: true,
    },
    BuffOption {
        slot: 2,
        cost: 0,
        stat_type: 1,
        value: 5,
        is_percent: false,
    },
    BuffOption {
        slot: 3,
        cost: 0,
        stat_type: 1,
        value: 10,
        is_percent: true,
    },
    BuffOption {
        slot: 4,
        cost: 0,
        stat_type: 2,
        value: 2,
        is_percent: false,
    },
    BuffOption {
        slot: 5,
        cost: 0,
        stat_type: 3,
        value: 1,
        is_percent: false,
    },
    BuffOption {
        slot: 6,
        cost: 0,
        stat_type: 4,
        value: 5,
        is_percent: false,
    },
    BuffOption {
        slot: 7,
        cost: 0,
        stat_type: 5,
        value: 1,
        is_percent: false,
    },
    BuffOption {
        slot: 8,
        cost: 0,
        stat_type: 6,
        value: 1,
        is_percent: false,
    },
    BuffOption {
        slot: 9,
        cost: 0,
        stat_type: 7,
        value: 1,
        is_percent: false,
    },
];

static CAPTURED_BUFFS: OnceLock<Mutex<Vec<BuffOption>>> = OnceLock::new();

fn buff_store() -> &'static Mutex<Vec<BuffOption>> {
    CAPTURED_BUFFS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn clear_captured_buffs() {
    if let Ok(mut buffs) = buff_store().lock() {
        buffs.clear();
    }
}

pub fn upsert_captured_buff(buff: BuffOption) {
    if let Ok(mut buffs) = buff_store().lock() {
        let slot = buff.slot.max(0) as usize;
        if slot >= buffs.len() {
            buffs.resize(slot + 1, BuffOption::default());
        }
        buffs[slot] = buff;
    }
}

pub fn captured_buffs_snapshot() -> Vec<BuffOption> {
    buff_store()
        .lock()
        .map(|buffs| buffs.clone())
        .unwrap_or_default()
}

pub fn known_buffs_snapshot() -> Vec<BuffOption> {
    let mut buffs = KNOWN_BUFFS.to_vec();
    let captured = captured_buffs_snapshot();
    for cap in captured {
        if let Some(slot) = buffs
            .iter()
            .position(|b| b.stat_type == cap.stat_type && b.is_percent == cap.is_percent)
        {
            buffs[slot].cost = cap.cost;
            if cap.value != 0 {
                buffs[slot].value = cap.value;
            }
        } else {
            buffs.push(cap);
        }
    }
    buffs
}

pub fn buff_kind_name(stat_type: i32, is_percent: bool) -> &'static str {
    match (stat_type, is_percent) {
        (0, false) => "HP Flat",
        (0, true) => "HP Percent",
        (1, false) => "ATK Flat",
        (1, true) => "ATK Percent",
        (2, _) => "Move Speed",
        (3, _) => "Rapid Fire",
        (4, _) => "Bullet Speed",
        (5, _) => "Full Heal",
        (6, _) => "Shield",
        (7, _) => "Extra Bullets",
        _ => "Unknown Buff",
    }
}

// Getter/setter tiện cho overlay (f32 ↔ bits).
pub fn speed_value() -> f32 {
    SPEED_VALUE.get()
}
pub fn set_speed_value(v: f32) {
    SPEED_VALUE.set(v)
}
pub fn fire_rate_value() -> f32 {
    FIRE_RATE_VALUE.get()
}
pub fn set_fire_rate_value(v: f32) {
    FIRE_RATE_VALUE.set(v)
}
pub fn bullet_speed_value() -> f32 {
    BULLET_SPEED_VALUE.get()
}
pub fn set_bullet_speed_value(v: f32) {
    BULLET_SPEED_VALUE.set(v)
}

// ──────────── Con trỏ method của các power đã resolve ────────────
/// (methodPointer, MethodInfo*) cho một managed method có thể gọi trực tiếp.
#[derive(Clone, Copy)]
pub struct Callable {
    pub code: usize,
    pub method: usize,
}

#[derive(Default)]
pub struct Powers {
    pub revive: Option<Callable>,
    pub add_exp: Option<Callable>,
    pub add_item: Option<Callable>,
    pub get_max_hp: Option<Callable>,
    pub save_game: Option<Callable>,
    pub shield_on: Option<Callable>,
    pub shield_off: Option<Callable>,
}

/// Được set một lần lúc install hook; hook Update đọc để gọi power.
pub static POWERS: OnceLock<Powers> = OnceLock::new();

/// Báo cáo hook (set sau khi install xong) — ghi ra log.
pub static HOOK_REPORT: OnceLock<String> = OnceLock::new();

// ──────────────── f32 atomic qua bit pattern ────────────────
pub struct AtomicU32f(core::sync::atomic::AtomicU32);
impl AtomicU32f {
    pub const fn new(v: f32) -> Self {
        Self(core::sync::atomic::AtomicU32::new(v.to_bits()))
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}
