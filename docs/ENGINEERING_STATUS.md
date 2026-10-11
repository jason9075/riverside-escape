# 工程狀態

- 更新日期：2026-10-11
- 目前階段：P0–P4 原型工程驗證完成；P4 三批功能、測試與雙瀏覽器整合驗證均已完成，待 PM 產品驗收
- 範圍：P0／P1／P2 是已驗收基線；P3 依 PM 最新決策完成，參數仍屬可調原型配置

## Git 基線與提交狀態

- 本機 `master` 起初沒有任何 commit，因此無法可信地重建 P0、P1、P2 各自的歷史提交；沒有捏造過往提交。
- 初始基線 commit 以已驗證的遠端 P2 checkout 為程式碼來源，納入 P0／P1／P2 原型、可重現驗證紀錄／截圖、最新 `PRODUCT_PLAN.md` 與本狀態文件，作為一個可檢視的目前狀態快照；未驗證中的 P3 程式不納入基線。
- 初始基線 commit：`5997679` (`chore: establish validated P0-P2 project baseline`)。
- 後續以可獨立驗證的功能切片提交；每項功能通過必要驗證並建立 commit 後，才標為已交付。Git 主 checkout 是本機，遠端工作副本不含 `.git/`。

## 已交付

- 建立共享座位、階段、輸入／快照協定及 Arcade Kinematic 自行車運動；Client 預測／離線預覽與 Server 共用 `BikeState::step` 和障礙檢查。
- P0 場景含自行車、Rider、Gunner、河濱道路、物件及障礙。Rider 可踩踏、煞車、轉向、右鍵自由看並回到追蹤鏡頭；Gunner 可旋轉後座並使用第一人稱鏡頭。
- P1 WebSocket Server 以 60 Hz 推進權威車體、20 Hz 傳快照，分配 Rider／Gunner；一般雙人模式仍要兩座各自 Ready，再由 Server 倒數五秒。
- 開發用單人模式需 Server 明確設定 `RIVERSIDE_DEV_SOLO=1`，頁面使用 `?solo=1`。單一連線佔用兩座，`V` 請求切換座位並由 Server 回覆確認；`R` 令兩座 Ready，再走原本五秒倒數。切座會清除 Rider 輸入，Server 只接受當前座位的輸入。
- HUD 顯示座位、Ready、倒數、車速、碰撞提示及 Client 校正距離。

## P2 首批攻擊原型

- Server 只在 `Running` 啟動一架直升機，並以 60 Hz 模擬 `Chasing → Telegraph → RocketInFlight → Repositioning`。一輪只有一枚火箭；預告時固定落點與範圍，爆炸時依自行車當下位置做 XZ 距離判定。等待、Ready、倒數不攻擊，重設／斷線會清除直升機、火箭、預告、事件與命中次數。
- 所有玩法暫定值集中在 `riverside-common`：

| 設定 | 暫定值 |
|---|---:|
| 追擊、預告、飛行、重新定位 | 3.0、2.0、0.8、2.0 秒 |
| 追擊距離、重新定位偏移、高度、速度 | 14、8、8 公尺、9 m/s |
| 落點速度預估時間、爆炸半徑 | 0.6 秒、2.5 公尺 |
| 戰鬥事件快照保留時間 | 5 秒 |

- `WorldSnapshot` 同時攜帶直升機／火箭狀態、命中計數，以及具 Attack ID 和單調 Event ID 的預告、發射、爆炸事件。Server 在快照短期重複事件，Client 依 Event ID 去重，確保晚到快照可恢復提示而不重複播放／結算。五秒保留窗涵蓋完整攻擊週期及落地後延遲。
- Client 使用既有 `helicopter.glb` 與 `missile.glb`。Rider 顯示固定落點紅色危險區及剩餘時間；Gunner 顯示威脅方向和倒數；兩端在命中／閃避後顯示 Server 結果與命中次數。此階段不加入血量、勝敗、鎖定、反擊或新資產。
- 新增 Client 啟動測試，確保各視覺 Query 互斥；Server 測試涵蓋倒數前不攻擊、留在落點只命中一次、騎離固定落點成功閃避、事件 ID 唯一／重設後不重用、事件落地後保留，以及 Rider／Gunner 座位權限。既有 common 固定步長 reconciliation 矩陣也持續通過。

## P3 鎖定反擊與勝敗循環

- Gunner 右鍵持續瞄準、左鍵單發；座椅可環繞 360°，俯仰限制 −45°～+75°。HUD 的 6° 鎖定環使用明確的 60° 相機垂直 FOV，預設隱藏診斷（F3 切換），避免工程數值遮住目標。
- Server 以當下世界狀態判定 120 m 範圍、視線遮蔽、連續 2 秒有效鎖定及 0.2 秒短暫偏離寬限；放開瞄準、超距、遮蔽或超出寬限後清鎖。接受開火前再次驗證座位、遊戲階段、輸入新鮮度、瞄準、鎖定、冷卻、彈藥與飛彈狀態。只有受理才扣除每局 6 發及啟動 2 秒冷卻；飛彈由 Server 導引、命中或逾時只結算一次。
- 修正 Gunner 相機／鎖定／飛彈發射點的後座座標一致性。開火時 Client 先將當幀瞄準輸入排在射擊請求前送出，降低固定 RTT 下舊的 20 Hz 輸入先被 Server 判為逾時的競態。
- Server 權威處理直升機 100 HP、玩家飛彈每次 34 傷害、腳踏車 100 HP 與敵方火箭每次 25 傷害。玩家飛彈速度 42 m/s、命中半徑 1.75 m，六秒未命中即逾時；命中或逾時均只結算一次。相同 Tick 同時歸零時腳踏車失敗優先。Finished 後停止攻擊並清除 Ready；雙座重新 Ready 時清理上一局戰鬥狀態與車體，再倒數五秒。Solo 的 `R` 強制 Ready／開始與 `V` 換座保留，切座不重補血量或彈藥。
- P3 數值集中在 `riverside-common`，對應 `PRODUCT_PLAN.md` 的暫定原型配置；不代表正式平衡決定。障礙碰撞傷害、終點、正式局長及命中回溯均不屬本批。
- P3 測試新增 Server 案例涵蓋 2 秒鎖定、寬限續接／清零、放開清鎖、座位／Tick／階段／新鮮輸入驗證、超距／遮蔽／錐外拒絕、重複請求、彈藥與冷卻、實際命中與唯一結算、重賽清理、切座不補給及同 Tick 失敗優先。Workspace 共 27 項測試通過（Client 3、common 3、reconciliation 6、Server 15）。
- 遠端實際 WASM／WebGL2 雙瀏覽器驗證注入固定 100 ms RTT（各方向 50 ms，未加入 jitter／loss）。兩端 Ready 後運行；Gunner pointer lock 生效，Server 回報完成鎖定並受理三發，直升機血量由 100→66→32→0；Rider／Gunner 在相同 Event ID 與 Tick 收到各發命中及 `riders_victory`，結果皆為直升機 0 HP、腳踏車 75 HP、Ready 清除。兩座重新 Ready 後倒數回到 Running，血量均重設為 100、彈藥 6；Solo 再按 `R` 開始、`V` 由 Rider 切 Gunner，血量與彈藥維持 100／6。三個瀏覽器均無 JS exception 或 console error。
- 驗證報告與畫面：[P3 驗證說明](verification/p3/README.md)、[雙端事件／血量報告](verification/p3/smoke-report.json)、[Gunner 看見直升機](verification/p3/gunner-helicopter-visible.png)、[Gunner 飛彈命中](verification/p3/gunner-missile-hit.png)、[Rider／Gunner 勝利畫面](verification/p3/rider-victory.png)、[Solo 換至 Gunner](verification/p3/solo-gunner-after-switch.png)。
- `cargo fmt --all -- --check`、`cargo test --workspace`、`cargo check --workspace`、WASM target check、`cargo clippy --workspace --all-targets`、Server build 與 Trunk Release build 均通過。
- P3 功能與程式、測試、瀏覽器報告及畫面證據一併提交。

## P4 第一批：共用河濱路線與路面碰撞

- 新增由 `riverside-common` 建立且快取的 `RiversideCourse`，以約 1.2 km 單一路線依序連接開闊河岸、橋下直線段及狹窄彎道。路段共 137 個 8 m 直線模組、20 個半徑 10 m／30° 彎道模組；左彎由既有右彎 GLB 反向通行，不需鏡射模型。Client 由同一批模組資料放置道路和彎道護欄，Server 不載入渲染素材。
- 路線總長 **1,200.7 m**：開闊河岸 401.9 m、橋下區段 400.0 m、狹窄彎道 398.8 m。彎道中心線以 0.5 m 間距取樣；自由騎乘控制不吸附中心線，車體跨出路面時由共用路面投影／寬度資料阻擋移動。取消舊的全域 X 座標短跑道邊界。
- Client 預測與 Server 權威皆呼叫同一 `BikeState::step`，碰撞查詢直接使用同一份 `riverside_course()`。固定步長受控 Rider 以持續踩踏及轉向追蹤全線，在避開原有水泥障礙後走到距離終點 3 m 內：**1,197.9 m／110.6 s／6,633 ticks**。這是工程控制器路線可通行證據，終點授權與正式逃脫時間會在檢查點／結算批次完成後重測。
- 新增測試涵蓋三段順序與路線長度、全部道路模組接縫、彎道兩側路面邊界及完整固定步長路線。`cargo test --workspace` 通過 31 項（Client 3、common 7、reconciliation 6、Server 15）；WASM target `cargo check`、workspace Clippy 與 Trunk build 通過。
- 遠端 Headless Chromium／SwiftShader 實際載入 WebGL2 場景，能看到 P4 路面與彎道；固定 100 ms RTT 雙 Client 的 P3 戰鬥回歸也通過，三發事件與勝利快照雙端一致、重賽重設及 Solo Ready／換座正常，三個 Client 無 JS 或 console error。此回歸讓 Rider 停在起點以分離移動對瞄準的影響；連續踩踏的原 P3 測試腳本會撞上舊水泥障礙並在第二發時失鎖，因此該腳本不作本次瞄準穩定性的驗收依據。
- 固定 100 ms RTT 的雙端 WASM 路線、橋下掩體與逃脫整合驗收在 P4 第三批完成；本批仍不代表實體 GPU、正式平衡或玩家體驗已驗收。可重現資料與畫面見 [`P4 路線驗證`](verification/p4/README.md)。
- 本批程式、測試、驗證紀錄與狀態一併獨立提交；commit hash 由 Git 歷史記錄。

## P4 第二批：橋下掩體與攻擊碰撞

- 共用課程資料在 400 m 橋下段生成 13 個橋面與 26 個橋墩碰撞體；橋面寬 13 m、底部高 4.9 m、頂部高 5.7 m，沿路面分跨鋪設。Client 依相同資料放置橋面和既有橋墩模型，Server 不載入渲染素材。
- Gunner 視線／鎖定與玩家飛彈路徑會被橋面或橋墩遮擋；Server 在橋下取消直升機預告並轉入重新定位，敵方火箭撞上橋面只記未命中而不傷車。直升機移動也會檢查下一段路徑，避免穿入碰撞體。道路高度以下的車道不會被橋面阻擋。
- 新增 common 與 Server 測試，覆蓋橋面 LoS／射線交點、可通行車道、掩體中的鎖定與開火拒絕、攻擊預告取消、雙方飛彈撞橋不造成傷害，以及直升機不穿越橋面。Workspace 共 **35 項**（Client 3、common 8、reconciliation 6、Server 18）；格式檢查、WASM target check、workspace Clippy、Server build 與 Trunk build 通過。
- 第一批的固定 100 ms RTT P3 戰鬥回歸沒有駛入橋下，因此不作掩體驗收依據；橋下 WASM 場景、區段路線及逃脫整合已在 P4 第三批完成。此批仍不代表實體 GPU 或戰鬥平衡驗收；測試範圍見 [`P4 掩體驗證`](verification/p4/README.md)。
- 本批程式、測試與工程紀錄獨立提交；commit hash 由 Git 歷史記錄。

## P4 第三批：檢查點、權威逃脫與完整路線驗收

- Server 記錄兩個依序通過的檢查點，僅接受在道路寬度內、沿路線前進方向的實際車體線段穿越；跳過、倒退、從檢查點側邊通過或直接越過終點均不會逃脫。通過兩點後再正向越過終點才結算 `riders_victory / escaped`；直升機擊落仍使用獨立的 `helicopter_destroyed` 勝利原因，同 Tick 腳踏車生命歸零優先判負。
- 路線 HUD 顯示目前區段、剩餘距離及檢查點進度；場景加入青色檢查點拱門與金色終點拱門。Finished 後 Rider／Gunner 分別顯示逃脫或擊落直升機。重玩會清除路線進度、勝利原因、傷害與彈藥狀態。Gunner 瞄準十字與鎖定環置於 1280 × 657 視窗中央；已檢視最新 WebGL2 截圖，兩者同心，HUD 分隔符使用 ASCII 以避免字型缺字。
- 固定步長全路線控制測試到達 1,199.8 m；20 Hz Rider 輸入加上合成 100 ms 回饋延遲的權威 Server 測試在 **121.7 秒／7,304 ticks** 逃脫。這是受控工程路線時間，不是玩家平衡數據。
- 遠端 Headless Chromium／SwiftShader WebGL2 的兩個 WASM Client 透過 50 ms 單向延遲代理（固定 100 ms RTT）實際走完三段路線。兩端在相同 Event ID 352 與 Server Tick 173,884 收到逃脫事件；路線長 1,200.7 m、通過兩個檢查點，逃脫時腳踏車 75 HP，Server Tick 換算運行時間 **208.5 秒**。橋下開火請求以相同 Event ID 333 被兩端拒絕為 `obstructed`，彈藥仍為 6；三個區段轉換、檢查點、勝利與重玩狀態都記入 JSON 報告。雙 Client 在 section 邊界的最新接收 Tick 可能因固定 RTT 短暫不同，完成事件的 ID／Tick 則一致。
- 重玩後路段、距離與檢查點回到起點／0，車輛與直升機生命均為 100，彈藥回到 6；Solo `R` 強制 Ready／開局、`V` 切到 Gunner 後維持 Running，沒有補給或瀏覽器錯誤。最新報告與截圖見 [`P4 完整驗證`](verification/p4/README.md)。
- 最終 Workspace 通過 39 項測試（Client 3、common 9、reconciliation 6、Server 21）、`cargo fmt --all -- --check`、WASM target check、workspace Clippy、Server build 與 Trunk build。固定 RTT 測試未注入 jitter 或 packet loss，SwiftShader 也不代表實體 GPU、最終平衡或正式玩家體驗。
- 本批程式、測試、報告與畫面證據獨立提交；commit hash 由 Git 歷史記錄。

## 遠端開發與驗證環境

| 項目 | 設定 |
|---|---|
| 本機 checkout | `/home/jason9075/Projects/river-side-escape` |
| 遠端主機 | `jason9075@10.90.0.254`（LAN；Tailscale `100.102.19.59`） |
| 遠端專案 | `/home/jason9075/hdd/side-projects/river-side-escape` |
| 同步方向 | 本機 checkout → 遠端專案；遠端執行 build 與測試，不回同步 `target/`、`dist/` |
| 遠端 Cargo 快取 | `/home/jason9075/.cache/riverside-escape/target` |
| 遠端日誌 | `/home/jason9075/.cache/riverside-escape/logs/` |
| 遠端工具 | Linux 7.0.0-38-generic、AMD Ryzen 9 5950X（32 vCPU）、Rust/Cargo 1.98.1、Trunk 0.21.14、Node 18.19.1、Chromium 154.0.8037.97 |

本次同步採單向 rsync，不使用刪除選項：

```sh
rsync -av --exclude='.git/' --exclude='target/' --exclude='dist/' ./ \
  jason9075@10.90.0.254:/home/jason9075/hdd/side-projects/river-side-escape/
```

遠端 build／驗證：

```sh
cd /home/jason9075/hdd/side-projects/river-side-escape
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HOME/.cache/riverside-escape/target"
cargo fmt --all -- --check
cargo check --workspace
cargo check -p riverside-client --target wasm32-unknown-unknown
cargo clippy --workspace --all-targets
cargo build -p riverside-server
trunk build
```

遠端啟動與停止命令（`trunk serve` 會建置 WASM 頁面；單人測試時才需要 solo 環境變數）：

```sh
ROOT=/home/jason9075/hdd/side-projects/river-side-escape
CACHE=/home/jason9075/.cache/riverside-escape
LOG="$CACHE/logs"
mkdir -p "$LOG"
nohup env RIVERSIDE_BIND=0.0.0.0:3001 RIVERSIDE_DEV_SOLO=1 \
  "$CACHE/target/debug/riverside-server" >"$LOG/server.log" 2>&1 </dev/null &
echo $! >"$LOG/server.pid"
nohup env CARGO_TARGET_DIR="$CACHE/target" PATH="$HOME/.cargo/bin:$PATH" \
  "$HOME/.cargo/bin/trunk" serve --address 0.0.0.0 --port 8080 \
  >"$LOG/trunk-serve.log" 2>&1 </dev/null &
echo $! >"$LOG/trunk.pid"

# 停止這兩個遠端服務
kill "$(cat "$LOG/trunk.pid")" "$(cat "$LOG/server.pid")"
```

測試時遠端 Server 綁定 `0.0.0.0:3001` 並開啟 solo，Trunk 在 `0.0.0.0:8080`。驗證後保留這兩項服務供 PM 瀏覽，PID 記錄在 `logs/server.pid` 和 `logs/trunk.pid`。頁面載入本次 Trunk build 的 WASM，`http://10.90.0.254:3001/health` 回覆 `ok`。本機與遠端同步本次 common、client、server 來源；遠端格式檢查、workspace 測試／check、WASM check、Clippy、Server build 與 Trunk Release build 均通過。

## P1 瀏覽器與畫面驗證

以下皆使用遠端 Chromium 154 headless／SwiftShader 載入遠端 WASM 頁面，沒有以本機瀏覽器代替：

- 單人 `?solo=1`：收到 `Welcome(solo_mode=true, seat=rider)`；按 `R` 觀察到雙座 Ready、Countdown、Running；`V` 兩次都收到 Server 的 Rider／Gunner `SeatChanged` 確認。右鍵測試先定位游標，再按住並小幅移動；自由視角改變，放開後約 1.7 秒回到 chase view，回正畫面與起始畫面一致。瀏覽器沒有 JavaScript runtime exception。
- 雙人兩個獨立 WASM 瀏覽器：Server 分配 Rider 與 Gunner；兩端 Ready 後進入 Countdown／Running。Rider 踩踏後權威車體移動，Gunner 的道路、路燈與樹木相對視角明顯位移，HUD 車速也更新，確認 Gunner 視角跟隨自行車移動。
- 視覺記錄： [Rider chase](verification/p1/rider-right-mouse-chase.png)、[右鍵自由視角](verification/p1/rider-right-mouse-free-look.png)、[右鍵放開回正](verification/p1/rider-right-mouse-recentered.png)、[50 ms Rider 校正 HUD](verification/p1/rider-50ms-correction.png)、[雙人 Gunner 移動前](verification/p1/two-browser-gunner-before.png)、[雙人 Gunner 移動後](verification/p1/two-browser-gunner-after-movement.png)。

## WebSocket 協定冒煙測試

此項與上方 WASM 實際畫面操作分開記錄。Node WebSocket client 對遠端 Server 驗證：一般雙連線依序取得 Rider／Gunner，雙座 Ready 後進入 Countdown／Running；solo 單一連線佔用兩座，`ForceStartSolo` 後雙座 Ready 並倒數，兩次 `SwitchSeat` 都收到正確座位確認；切回 Rider 傳入踩踏後收到移動中的權威快照。普通雙人分配和 Ready 流程仍正常。

## P2 雙瀏覽器 WASM 與 100 ms 驗證

兩個隔離的遠端 Headless Chromium 154／SwiftShader 載入本次 Trunk WASM，使用 WebSocket 代理注入固定 100 ms RTT（兩方向各 50 ms；此為應用層 TCP chunk 排程，不是 qdisc；jitter 與 loss 為 0／未注入）：

- Rider、Gunner 分別按 `R` Ready；Server 倒數五秒後才進 Running，兩個 Client 在相同快照 Tick 收到相同 Attack ID、固定落點、impact Tick 與 2.5 m 半徑。Rider 顯示紅色固定危險區與倒數，Gunner 顯示 `INCOMING ROCKET: CENTER` 與相同倒數。
- 留在區內時，兩端在同一快照 Tick 收到相同爆炸 Event ID，Server 結果皆為 hit、`hit_count = 1`。同一 Attack ID 的警告／發射／爆炸 Event ID 順序與數值雙端一致，沒有重複事件。Server 單元測試另驗證移出固定區域可成功閃避。
- 雙人 Ready／倒數／Running、同 Tick 權威自行車狀態與 Rider input Ack 正常；Server 座位權限測試拒絕 Gunner 騎車、拒絕 Rider 操作 Gunner，fixed-step reconciliation 矩陣通過。solo 瀏覽器按 `R` 強制兩座 Ready、完成倒數，再按 `V` 並收到 Server 確認切至 Gunner。三個 Client 均無 JavaScript runtime exception，WebGL2 canvas 尺寸為 `1280 × 657`。
- 視覺記錄：[Rider 預告區與倒數](verification/p2/rider-rocket-warning.png)、[Gunner 威脅方向](verification/p2/gunner-rocket-warning.png)、[Rider 命中提示](verification/p2/rider-rocket-hit.png)、[Gunner 命中次數](verification/p2/gunner-rocket-hit.png)；[雙端快照與事件明細](verification/p2/smoke-report.json)。

此為遠端 SwiftShader 畫面與固定延遲原型驗證，不代表實體 GPU、不同瀏覽器、jitter、packet loss 或實際玩家體驗已驗收。P2 本批沒有進入 P3。

## RTT 延遲基線

| 設定 RTT | 輸入送出至第一個移動權威快照 | 快照間隔 median／p95 | Rider HUD 最大校正距離 | 應用層快照缺口 |
|---:|---:|---:|---:|---:|
| 50 ms | 80 ms | 50.2／53.7 ms | 約 1.55 m | 0 |
| 100 ms | 131 ms | 50.1／52.5 ms | 約 1.42 m | 0 |
| 200 ms | 252 ms | 50.3／52.4 ms | 約 1.54 m | 0 |

量測定義與限制：

- `輸入至移動快照` 是 Chromium DevTools 記錄到 RiderInput 踩踏訊息送出，到收到 `speed > 0.05 m/s` 的第一個 Running 權威快照的時間差。它包含傳輸延遲、Server 60 Hz 推進及 20 Hz 快照時序，**不是 raw RTT 測量**。
- 固定延遲由 `tools/ws_latency_proxy.js` 注入：遠端 Chromium 明確把遊戲 WebSocket 指向遠端 loopback 的 3101／3102／3103，再由代理轉送至 `127.0.0.1:3001`。兩個方向各延遲設定 RTT 的一半（25／50／100 ms），沒有使用 `tc`、qdisc 或修改主機其他流量；代理只監聽 `127.0.0.1`，Trunk 靜態頁面連線未經代理。
- 此代理對 WebSocket TCP byte stream 的資料 chunk 加固定排程延遲，是遊戲連線專用的固定延遲近似；不是網卡封包層 shaping。Jitter 設定 0 ms、沒有注入 packet loss。快照 tick 間距為 3（符合 60 Hz／20 Hz），大於預期間距的應用層缺口數為 0；這不代表測得 TCP loss 為零。
- **Jitter 與 TCP packet loss 未測**。沒有用應用層丟訊息冒充封包遺失，也沒有影響遠端主機其他服務。RTT 三組測試時序和 HUD 校正值是基線，不能視為不同 RTT 下的操控調校結果。

## PM 追問：prediction、輸入確認與 Gunner 轉向校正（首輪診斷）

為釐清 HUD 數字並觀察 Rider 快速左右轉時 Gunner 視角的額外跳動，加入快照 tick／Rider 輸入確認序號、未確認 Rider 輸入數、實際套用的每幀校正量、轉向後短時間窗的 Gunner 校正峰值，以及最大 app frame delta。`Pred↔snapshot` 是該畫面目前 Client 車體位置與最新 Server 權威快照位置的距離；HUD 同時列出快照 tick，並明示兩者 tick 不同。它是跨時間的狀態差，不是同一 tick 的預測誤差，也不代表車體在該畫面瞬間跳了相同距離。`Applied/frame` 才是本畫面這一幀由權威平滑校正造成的車體位移與 Gunner 相機位置／角度變化；`Turn-window` 是權威 yaw 改變超過 0.01 rad 後 150 ms 內的 Gunner 校正最大值。

Server 現在在每個快照附上 `last_applied_rider_sequence`。Client 依確認序號清除已確認的 Rider 輸入，HUD 顯示剩餘筆數及最大佇列深度。這個佇列用來觀察輸入是否已由 Server 接受並推進，不會回滾或重播歷史輸入。現行 Rider 預測仍以當前按住的輸入持續推進，再平滑靠近權威快照；WebSocket 為有序可靠傳輸，因此目前採 held-state prediction。若未來改用不可靠傳輸，才需以 tick／序號保存歷史輸入並做 rollback/replay。100 ms 首輪截取時 12 筆中有 1 筆尚未在 trace 收到確認；用相同路線重跑並延長閒置觀察後，17/17 筆均已確認，最後送出與最後確認序號皆為 25，p95 約 145 ms。200 ms 的 WebSocket trace 為 12/12 確認；其中一張低幀率 HUD 截圖仍顯示一筆 pending，表示畫面狀態落後於 DevTools 記錄，不代表 trace 中輸入未確認。

固定路線每組皆先等待 8 個穩定快照確認起始車體在預設位置，再踩踏、短暫 D／A 轉向、鬆開並煞車；沒有座位切換、碰撞或 reset。以下輸入確認和畫面校正數字取自修正後的 headless Chromium／SwiftShader 觀察；轉向窗由 Rider yaw 變化後 150 ms 定義：

| 設定延遲 | Rider 輸入確認 | 確認延遲 median／p95 | Rider 最大 Pred↔snapshot 跨 tick 差 | Gunner 每幀校正最大值 | 轉向窗 Gunner 校正最大值 | 最大 app frame delta |
|---:|---:|---:|---:|---:|---:|---:|
| 0 ms | 12/12 | 16.4／48.9 ms | 4.46 m | 0.560 m／0.99° | 0.337 m／0.99° | 250 ms |
| 50 ms | 12/12 | 68.2／98.1 ms | 4.40 m | 0.491 m／0.97° | 0.310 m／0.97° | 256.6 ms |
| 100 ms | 首輪 11/12；延長觀察重跑 17/17 | 重跑 122.1／145 ms | 4.34 m | 0.494 m／1.06° | 0.313 m／1.06° | 250 ms |
| 200 ms | 12/12 | 219.3／244.4 ms | 3.20 m | 0.384 m／1.40° | 0.292 m／1.40° | 250 ms |

發現校正因子原先直接使用完整 render-frame delta；當 headless frame 停頓時，單幀可能一次平滑多個模擬 tick 的距離。已將權威位置、yaw、速度校正使用的 delta 限制在最多一個 60 Hz 模擬 tick，且保留實際校正量的量測。相同路線修正前 Gunner 單幀最大校正約 0.85–1.305 m，修正後約 0.384–0.560 m；修正後快速轉向時間窗最大約 0.292–0.337 m。這表示 headless 量測中的額外單幀跳動明顯降低，但跨 tick 的 `Pred↔snapshot` 差仍達數公尺，沒有被修正或隱藏。實體 GPU／一般瀏覽器幀率尚未測，因此不能由此判定玩家實際裝置上的 Gunner 手感已驗收。

這批 headless app frame delta 約 250–257 ms，且約 4.8 秒路線只送出 12 筆輸入，遠低於 20 Hz 設計更新率；這是 SwiftShader 渲染／自動化測試環境的限制，讓平滑效果和 HUD 顯示可能落後，不能當作 60 FPS 畫面評估。固定延遲由專用 loopback WebSocket 代理設定，未使用 qdisc；**Jitter、TCP packet loss 與實體 GPU 仍未測**。

另確認停車時 `BikeState::step` 仍會隨 A／D 改變整車 yaw，但不改變 x／z。Gunner 因跟隨整車 yaw，車停著轉向也會轉動整個視角。後續提案是把前叉／手把的視覺轉向角與車身／行進 yaw 分開；停車時 A／D 只轉手把，車身 yaw 僅在有速度時依速度及路徑曲率改變。這次只記錄提案，未改控制器，也不加入傾斜。

## PM 後續決策：tick reconciliation、停車轉向與固定時間呈現

依架構指南 v0.4 與 PM 最新決策完成以下變更。先前以 current held input 連續預測、只清 Ack 佇列、校正 delta 每幀封頂的方案已替換；首輪 headless 瀏覽器數字保留為歷史診斷，不作為本方案驗收值。

- Client Rider 以 60 Hz 固定步長推進，保留 300 個 tick 的輸入／狀態歷史。每個權威快照依 `snapshot.tick` 找同 Tick 預測，記錄位置及 yaw 誤差，將模擬狀態恢復成快照，再逐 tick 重播快照之後的本地輸入。每 20 Hz Rider／Gunner input 帶 Client tick 與 sequence；Client 保留尚未 Ack 的 Rider sequence、tick 與輸入內容。reconcile 時以 snapshot tick 和 Ack 為界，將仍未確認且位於快照之後的輸入覆寫到對應預測 tick，再重播到目前 tick；Client 依 Ack 清除已確認佇列項目。Server 拒絕超過 300 tick 的舊 input 及比 Server tick 超前超過 3 tick 的 input。
- 每個 Update 先補完固定步長預測，再依序處理排隊的權威快照，使低 FPS 一次收到多個快照時仍有可比較的同 Tick 狀態。快照 tick 超過 Client 預測時，從該權威 tick 重新設錨並繼續模擬；超出 5 秒歷史範圍時記錄 history miss 並重新設錨，不用不連續的歷史假裝完整 replay。
- Rider 視覺以 `BikeRenderOffset` 保留權威恢復前的畫面位置，offset 依實際經過時間以 85 ms time constant 衰減，沒有每幀 delta 上限。Gunner 使用 Server tick 對齊的快照緩衝，以最新估計 Server tick 往回 3 tick（50 ms）取樣並插值；僅在最新快照之後最多外推 6 tick。車輛移動和轉彎照常改變 Gunner 視角，這是玩法影響，不會被平滑移除。
- 共用 `BikeState::step` 改成獨立前叉／手把 steering angle 與車身 yaw。停車時 A／D 以連續速率轉前叉，x／z／yaw 不變；前進時車身依速度與轉向角連續改變 yaw。Gltf 的 `bicycle.Steering` 節點顯示前輪與手把角度。不加入 lean。

已新增無 WebGL 的固定步長測試，模擬 240 個 20 Hz input sequence、Server 60 Hz 與 20 Hz snapshot、Client 15／30／60 FPS 呈現時序、0／50／100／200 ms 固定 RTT，以及 30 FPS／100 ms RTT 下單次約 200 ms render stall。每組在起步、左右轉向、煞車停止後繼續觀察，直到輸入 Ack 排空。誤差是收到快照前，Client 歷史中同一 Server tick 的狀態與權威狀態差；收斂時間從 Server 接受放開輸入開始，直到連續三份權威快照的位置差低於 2 cm。Gunner lag 是模擬 Server 現在 tick 與實際插值取樣 tick 的差，已包含注入的單向延遲及 50 ms 插值緩衝。

| 呈現 FPS | RTT | 最大同 Tick 位置／yaw 誤差 | 放開後位置收斂 | 最大未確認輸入 | Ack | Gunner 實際狀態年齡（平均／最大） |
|---:|---:|---:|---:|---:|---:|---:|
| 15 | 0 ms | 0.005 m／0.53° | 183 ms | 2 | 240/240 | 50／50 ms |
| 15 | 50 ms | 0.005 m／0.53° | 167 ms | 3 | 240/240 | 75／75 ms |
| 15 | 100 ms | 0.005 m／0.49° | 150 ms | 3 | 240/240 | 100／100 ms |
| 15 | 200 ms | 0.005 m／0.49° | 233 ms | 5 | 240/240 | 150／150 ms |
| 30 | 0 ms | 0.005 m／0.27° | 150 ms | 1 | 240/240 | 50／50 ms |
| 30 | 50 ms | 0.005 m／0.27° | 167 ms | 2 | 240/240 | 75／75 ms |
| 30 | 100 ms | 0.005 m／0.45° | 150 ms | 3 | 240/240 | 100／100 ms |
| 30 | 200 ms | 0.005 m／0.52° | 200 ms | 5 | 240/240 | 150／150 ms |
| 60 | 0 ms | 0.005 m／0.09° | 133 ms | 1 | 240/240 | 50／50 ms |
| 60 | 50 ms | 0.005 m／0.27° | 150 ms | 2 | 240/240 | 75／75 ms |
| 60 | 100 ms | 0.005 m／0.45° | 150 ms | 2 | 240/240 | 100／100 ms |
| 60 | 200 ms | 0.005 m／0.53° | 200 ms | 4 | 240/240 | 150／150 ms |

12 組共 2,880 筆 Rider sequence 均獲 Server Ack，Server 未拒絕 tick，Client 沒有 history miss；結束時同 Tick 位置與 yaw 誤差皆小於 0.01。額外 replay 單元案例以 snapshot tick 104 恢復後重播 105–109：ack 停在 sequence 2 時，sequence 3 在 tick 107 的新輸入覆寫原預測並被重播，最終狀態回到 tick 109。停車轉向案例確認前叉角會變，但車身 x／z／yaw 不變；踩踏後車身 yaw 與位置連續變化。

修正 offset 的 5% 剩餘值收斂時間在 15／30／60 FPS 均為約 267 ms（理論 time constant 導出的 255 ms 加上呈現取樣量化）；200 ms 沒有 render frame 時 offset 仍按經過時間衰減，恢復後不需再等待固定 tick 封頂累積。快照插值緩衝讓 Gunner 狀態年齡穩定為 RTT/2 + 50 ms：在 0／50／100／200 ms RTT 分別為 50／75／100／150 ms，15／30／60 FPS 結果一致。

短停頓案例在 30 FPS／100 ms RTT 停止呈現 200 ms，恢復時最大 frame gap 約 233 ms；仍完成全部 240/240 Ack，pending 歸零、history miss 為零，位置誤差依序收斂，Gunner 狀態年齡回到 100 ms。完整 workspace 的 test、check、WASM check、Clippy、Server build 及 `trunk build` 均通過；Clippy warning 已清除。未啟用 WebGL 的 reconciliation 測試由 `cargo test -p riverside-common --test reconciliation_matrix -- --nocapture` 執行。

遠端 WASM 瀏覽器冒煙以兩個隔離的 Headless Chromium 分別連入 Rider／Gunner，兩席按 Ready 後進入 Running，8 份起始快照保持預設車體位置；Rider 踩踏、左右轉向及煞車後車體前進約 19.23 m，20 Hz snapshot tick 間距維持 3。該輪 protocol trace 在量測截點收集 105 筆 Rider input、其中 100 筆已 Ack，最後 5 筆仍 pending；後續 HUD 顯示 Client／Server tick 對齊、history miss 為 0，但低幀率畫面曾累積最多 35 筆 pending，最大 app frame delta 約 250 ms。這個 SwiftShader/headless 冒煙只確認頁面、雙席 Ready／Running、控制輸入與持續快照可運作；其輸入 Ack 延遲不作為穩態網路基線，時序驗收以 12 組固定步長矩陣為準。

## Drifter 操控研究對照

已閱讀使用者提供的 `07-bike-handling.md`，並對照架構指南 `6.3` 與目前實作。該資料包含機車專用數值與推論，僅供檢查方向；以下不移植其質量、速度、長度單位、轉向角／油門曲線、按鍵或傷害數值。研究評估與網路量測分開，不以 RTT 結果推論操控品質。

| 方向 | 評估 | 依據與後續提案 |
|---|---|---|
| 踩踏／煞車分離 | 符合 | 左鍵踩踏、空白鍵煞車；`BikeState::step` 對加速、煞車和鬆開後滑行分別處理。保留此分工，不把 Drifter 油門曲線套進按鍵。 |
| 共用 Kinematic | 符合 | Server 與 Client 預測／預覽使用同一個 `BikeState::step`，維持權威和預測規則一致。 |
| 速度敏感轉向 | 待量測 | 現行共用模型以 `yaw_rate = speed × 0.148 × steer_ratio` 更新車身方向，靜止時只轉前叉／手把，不改車身 yaw；滿舵的名目轉彎半徑約 6.76 m，速度主要改變角速度。尚無固定速度左右半徑與反向操作基線；後續可量測後再調共享曲線。 |
| 數位輸入漸變 | 需調整／待評估 | A／D 目前直接輸出 −1／0／+1，沒有方向輸入漸變；踩踏與煞車是數位輸入，但車速仍透過加減速度逐步變化。提案：先記錄按下、放開、左右切換的反應，再決定是否在共用模擬加入可調輸入變化率；不先增加油門／煞車機制，也不混淆網路平滑。 |
| 視覺傾斜與彈簧回正 | 需調整／待評估 | 現在車體只更新平移與 yaw，沒有 lean、回正彈簧或車手／頭部視覺層。提案：若測試需要，再做純呈現層的可調彈簧傾斜，不改權威 Kinematic；另測它對 Gunner 畫面的影響，傾斜影響轉向需獨立驗證。 |
| 分層鏡頭 | 符合（部分） | Rider 右鍵自由觀看並回到 chase；Gunner 第一人稱組合自行車 yaw 與座椅方向，兩人瀏覽器測試已確認車輛移動會改變 Gunner 世界視角。尚無車身／乘員／頭部彈簧分層；如要增加平滑，只在視覺層評估，不移除車手移動對 Gunner 的影響。 |
| 接觸傷害冷卻 | 暫不採用（P1） | 目前只有障礙碰撞停車與 HUD 提示，沒有生命值或傷害事件，現在無 cooldown 可驗。加入傷害時再由 Server 依傷害來源／目標限制重複觸發，避免每 Tick 傷害；不沿用 Drifter 冷卻秒數。 |
| Drifter 專用玩法／數值 | 暫不採用 | 不加入燃料、Nitro、漂移、排檔，不移植機車質量、極速、單位、按鍵、角度或傷害曲線。本專案維持腳踏車、踩踏／煞車和 Rider／Gunner 合作定位。 |

## 尚待驗收與下一步

- P3 已完成實作、必要驗證與功能 commit，待 PM 工程驗收；本批生命值、傷害、鎖定與武器值仍是 PM 可調原型配置，不是正式平衡結論。
- 固定 100 ms RTT 的遠端 SwiftShader／headless 驗證通過；jitter、packet loss、實體 GPU、其他瀏覽器與完整玩家體驗仍待後續驗收。
- P4 路線、橋下掩體與逃脫功能已完成工程驗證；正式戰鬥平衡及玩家體驗仍待 PM／玩家驗收。
- 控制器尚無量化轉彎測試；依研究對照先驗證問題，再決定是否調整轉向曲線、輸入漸變或視覺傾斜，不擴寫控制器。
- 完整 Lobby、重連與正式斷線規則留待後續階段。

