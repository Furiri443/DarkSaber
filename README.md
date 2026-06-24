# DarkSaber

DarkSaber là một dự án cheat cho game `Echo` của DevG. Nó tạo `echo.dll` để can thiệp IL2CPP trong `GameAssembly.dll` của Unity x64, kèm theo overlay riêng để bật/tắt các chế độ hack/chỉnh sửa chỉ số.

## Tổng quan

- `echo.dll` là modul DLL probe/mod, được nạp vào tiến trình game.
- `darksaber_loader.exe` là loader an toàn, inject DLL bằng `LoadLibraryA` thay vì `CreateRemoteThread`.
- Dự án tạo overlay GDI riêng, không hook đồ họa game, nên tương thích với DX11/DX12/Vulkan/OpenGL và Wine.

## Cơ chế hoạt động

1. `DllMain` của `echo.dll` chạy và spawn thread bootstrap.
2. Bootstrap chờ `GameAssembly.dll` có IL2CPP runtime sẵn sàng.
3. Attach thread vào VM, resolve metadata và cài hook IL2CPP.
4. Nếu tồn tại `dump_request.txt`, dự án sẽ dump metadata vào file.
5. Overlay riêng được tạo bằng window layered, điều khiển bằng phím `\`.

## Tính năng chính

- Hook `PlayerController.Update` để chỉnh trực tiếp các field chỉ số.
- God Mode: nuốt sát thương trong `TakeDamage`.
- Max HP Override: hook `GetMaxHp` và ép giá trị max.
- Tăng tốc chuyển động, tốc độ đạn, giảm cooldown bắn.
- One-Hit Kill: tăng damage trong `EnemyManager.TakeDamage`.
- Add item / Add exp / Revive / Activate shield.
- Lock shield/HP và bắt instance shield bằng hook `PlayerShield.Start` / `Update`.
- Overlay menu chỉ hiển thị khi cần, không cướp focus game.

## Điều khiển overlay

- `\` : bật/tắt menu.
- `↑` / `↓` : chọn dòng.
- `←` / `→` : chỉnh giá trị hoặc bật/tắt.
- `Enter` : kích hoạt hành động.

## Build

Dự án dùng Rust `nightly` và target Windows x64. Thư mục `rust-toolchain.toml` đã chỉ định toolchain.

```bash
cd /Volumes/SoftwareDisk/Data/CodeP/DarkSaber
env CARGO_HOME=$PWD/.cargo cargo build --release --target x86_64-pc-windows-gnu
```

Kết quả:

- `target/x86_64-pc-windows-gnu/release/echo.dll`
- `target/x86_64-pc-windows-gnu/release/darksaber_loader.exe`

## Sử dụng

1. Sao chép `echo.dll` và `darksaber_loader.exe` vào thư mục game.
2. Chạy `darksaber_loader.exe` với tên executable game và tên DLL nếu cần.
3. Loader sẽ tạo tiến trình game tạm dừng, inject `echo.dll` và resume game.

Ví dụ:

```bash
darksaber_loader.exe Echo.exe echo.dll
```

## Lưu ý quan trọng

- Không dùng loader gọi remote export `StartIl2CppProbe` bằng `CreateRemoteThread` dưới Wine/CrossOver.
- `darksaber_loader.exe` inject bằng `LoadLibraryA` để tránh lỗi địa chỉ export 64-bit bị cắt.

## File chính

- `src/lib.rs` : bootstrap DLL, `DllMain`, export `StartIl2CppProbe`, ghi log.
- `src/hooks.rs` : cài hook IL2CPP, chỉnh chỉ số và xử lý power.
- `src/il2cpp.rs` : binding và resolve API IL2CPP runtime.
- `src/overlay.rs` : tạo overlay window layered, xử lý input và vẽ menu.
- `src/dumper.rs` : dump metadata IL2CPP khi có `dump_request.txt`.
- `src/state.rs` : lưu state toggle, instance, request power.
- `src/bin/darksaber_loader.rs` : loader safe inject DLL.
- `Cargo.toml` : cấu hình crate DLL và dependencies `retour`, `windows-sys`.

## Debug

- File log: `il2cpp_probe.log` sẽ ghi trạng thái bootstrap và overlay.
- Nếu overlay không hiện, kiểm tra xem game có đang chạy fullscreen độc quyền hay không.
- Nếu game cần dump metadata, tạo file `dump_request.txt` trong thư mục chạy để kích hoạt dump.

## Ghi chú

Dự án phù hợp cho nghiên cứu và thử nghiệm nội bộ. Sử dụng có trách nhiệm và chỉ với phần mềm bạn được phép can thiệp.