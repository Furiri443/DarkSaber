# echo.dll — IL2CPP trainer + overlay (Rust)

DLL Rust can thiệp `GameAssembly.dll` (Unity IL2CPP, x64) kèm overlay in-game
để bật/tắt can thiệp chỉ số và quyền năng.

## Cơ chế
API IL2CPP resolve **runtime** qua `GetProcAddress` (không hardcode RVA). Bootstrap
(`src/lib.rs`): chờ runtime → `il2cpp_thread_attach` → dump `il2cpp_dump.txt` →
cài hook → bật overlay.

## Overlay (tương thích Wine + Windows thật)
Cửa sổ layered GDI riêng, KHÔNG hook Present/SwapBuffers → chạy với mọi backend
(DX11/DX12/Vulkan/OpenGL) và Wine. Trong suốt, topmost, click-through, không cướp
focus. Điều khiển bằng phím qua `GetAsyncKeyState`.

Điều khiển: **`\`** mở/đóng menu · **↑/↓** chọn dòng · **←/→** chỉnh / bật-tắt ·
**Enter** kích hoạt power.

## Cách can thiệp (kiến trúc)
- **Chỉ số** = GHI THẲNG FIELD mỗi frame trong `PlayerController.Update` (RE cho thấy
  game đọc field trực tiếp, hook getter không ăn). Offset (IDA base 0x180000000):
  moveSpeed `+184`, bulletSpeed `+216`, fireRate `+220` (float); curHp `+56` (int).
- **God Mode** = hook `TakeDamage(Int32)` → `return` (nuốt sát thương) + bắt `this`.
- **Max/Lock HP** = khoá curHp `+56` = giá trị mỗi frame + nâng `GetMaxHp` return.
- **Rapid Fire** = chỉnh cooldown `fireRate +220`; nhỏ hơn là nhanh hơn (mặc định
  `0.05s`). Khi tắt sẽ restore giá trị gốc đã snapshot.
- **Fast Bullets/Speed** = snapshot field gốc rồi restore khi tắt, tránh lỗi ghi
  lặp làm trạng thái game bị kẹt.
- **Shield Lock** = ép shield HP của PlayerController `+104` và byte active của
  `PlayerShield +56`, đồng thời hook cả `PlayerShield.Start` và `Update` để bắt
  instance chắc hơn.
- **One-Hit Kill** = hook `EnemyManager.TakeDamage(Int32)` và nâng damage khi bật.
- **Item** = có AddItem theo ID, Add preset, và Add range `1..N`. Inventory gốc chỉ
  có giới hạn slot, nên các item ngoài giới hạn/không tồn tại sẽ bị game bỏ qua.
- **Powers** (Revive/AddExp/AddItem/Shield) = overlay set cờ request; `Update` hook
  (thread VM) tiêu thụ và gọi method với `this` + `MethodInfo*`. Shield instance bắt
  ở `PlayerShield.Start` (chạy lúc spawn dù shield bị tắt).

## File
- `src/state.rs` — cờ toggle (atomic) + instance bắt được + request power.
- `src/il2cpp.rs` — binding API; `method_address` / `resolve_method`.
- `src/dumper.rs` — `il2cpp_dump.txt` (chữ ký + địa chỉ IDA).
- `src/hooks.rs` — il2cpp hook + ghi field + pump power.
- `src/overlay.rs` — overlay cửa sổ layered GDI, điều khiển bằng phím.
- `src/lib.rs` — `DllMain` + export `StartIl2CppProbe`, logger `il2cpp_probe.log`.

## Build (macOS → Windows x64)
Toolchain pin **nightly** qua `rust-toolchain.toml` (cần cho `static_detour!`).
```bash
env CARGO_HOME=$PWD/.cargo cargo build --release --target x86_64-pc-windows-gnu
# → target/x86_64-pc-windows-gnu/release/echo.dll
# → target/x86_64-pc-windows-gnu/release/darksaber_loader.exe
```

## Nạp
- First-party: gọi export `StartIl2CppProbe`.
- Injection: dùng `darksaber_loader.exe` trong thư mục game; nó inject `echo.dll`
  bằng remote `LoadLibraryA`, rồi `DllMain` tự bootstrap.

Không dùng `loaderhonk.exe` dưới Wine/CrossOver. Loader đó gọi remote export
`StartIl2CppProbe` bằng `CreateRemoteThread`, nhưng địa chỉ export 64-bit bị cắt
còn 32-bit (`0x6FFFF8ADC570` → `0xF8ADC570`), khiến Unity nhảy vào vùng không
thuộc module nào và crash ngay.

## Overlay không hiện?
Xem `il2cpp_probe.log`:
- `[overlay] cửa sổ layered đã tạo` + `[overlay] tick đầu tiên` → overlay chạy (nhấn
  `\`; nếu game fullscreen độc quyền thì chuyển sang borderless/windowed).

> Dùng cho game của chính bạn / nghiên cứu được uỷ quyền.
