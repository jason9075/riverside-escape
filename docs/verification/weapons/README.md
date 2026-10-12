# 武器與動畫驗證 · 2026-10-12

本次只處理武器、人物與直升機動畫；地圖／路線由另一工作階段同時修改。

## 操作與預覽

- 1：原有鎖定發射器；右鍵維持鎖定後，左鍵發射。
- 2：霰彈槍；左鍵單發，可腰射；按住右鍵縮小 FOV、移至瞄具視角並收窄散布。
- Rider 踩踏時曲柄／雙腳交替運動；滑行停踩，車輪按速度持續轉動。
- 直升機主旋翼／尾旋翼旋轉、機身輕微擺動；兩位角色穿藍白拖鞋。

[發射器腰射姿態](launcher-hip.png) · [發射器 ADS](launcher-ads.png) · [霰彈槍腰射](shotgun-hip.png) · [霰彈槍 ADS](shotgun-ads.png) · [射擊後](shotgun-fired.png)

[踩踏姿勢 A](pedaling-a.png) · [踩踏姿勢 B](pedaling-b.png) · [放開踏板](coasting.png) · [Rider 報告](rider-report.json)

## 驗證結果

- `cargo test --workspace`：client 6、common 25、reconciliation 6、server 38，全部通過。涵蓋完整踩踏週期腳掌位置／方向、散布與遮蔽、伺服器權限／冷卻／彈藥／去重、切槍清鎖定及輸入逾時保留武器。
- `cargo clippy --workspace --all-targets`：通過，無警告。
- `NO_COLOR=true trunk build`：WASM / WebGL2 建置成功。
- 全部 16 個 GLB 通過 Blender 匯入回讀與 Khronos Validator，0 errors / 0 warnings。詳見 `art/validation/`。
- 真實瀏覽器連線至隔離測試伺服器（solo）：1／2 切換、ADS 與腰射各一次，霰彈 24 → 22，發射器維持 6 發；兩次各命中 8 顆彈丸，由伺服器結算。瀏覽器沒有執行例外或載入錯誤。詳見 [報告](browser-report.json)。

截圖使用 Chrome headless WebGL2／SwiftShader，不代表硬體 GPU 效能測試。動畫為程式驅動，GLB 沒有預烘焙動畫片段。
