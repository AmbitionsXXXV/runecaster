# Runecaster 工程评审（游戏开发视角）

> 评审日期：2026-07-06 ｜ 评审执行：opus-4.8 subagent（资深 Rust/Bevy 游戏程序角色）
> 验证：`cargo test -p rune_core` **32 passed / 0 failed**（README 写 ~20，已过时）；`cargo clippy -p rune_core --all-targets` 仅 1 个测试文件风格 warning（playthrough.rs:16，manual_range_patterns）
> （遵嘱未编译 runecaster/workspace，main.rs 纯人工阅读）

## 总体健康度

这是一个**健康度明显高于同规模个人项目**的代码库。逻辑/渲染分离执行得非常干净——`rune_core` 除 `rand` 外零依赖，Bevy 完全隔离在 `runecaster`，两者仅通过 `World` 上的两个事件缓冲（`fx`/`cast_fx`）+ 每步全量重建通信，渲染层从不反向写入 sim 内部状态。rune_core 几乎无 `unwrap`/panic 点，magic number 大部分已抽成顶部常量，核心 `match`（compile/apply_spell_to/element 映射）全部无 `_` 通配、有编译期完备性保护。真正的问题不在崩溃或数据损坏层面，而集中在**少数 gameplay 语义 bug（死字段、无差别治疗）**、**几处静默的可维护性陷阱（硬编码表绕过了 match 保护）**、以及 **main.rs 1370 行该拆分**。determinism 比 README 声称的要弱（只测了首状态）。

---

## 1. 架构

- 【优点】逻辑/渲染分离干净：`rune_core` 依赖仅 `rand`（crates/rune_core/Cargo.toml:7-8）；`Fx`/`CastFx`（world.rs:81-130）以纯数据携带渲染意图，sim 可测、前端无需重新推导。渲染层通过 `std::mem::take`（main.rs:426-427）抽取事件，这是唯一的"写"，合理。
- 【中等】**回合引擎无能量/速度系统**：`end_turn`（world.rs:885-908）是固定顺序，速度只有二元 `slowed`（跳过整回合）。要加"敏捷怪/多重行动/快慢速"必须重构成能量累加器制。属设计上限，非缺陷。
- 【中等】**Fx 抽象略薄，不带数值**：`Fx`（world.rs:90-98）没有 `amount`，渲染层想显示飘血数字做不到，只能像 `last_hp` 那样差分（main.rs:410-415）。若要飘血 UI，需给 Fx 加伤害字段。
- 【轻微】`resolve_spell`（world.rs:560）已按 `caster_id` 泛化、预留怪物施法，但 `AiKind` 只有 `Chaser`（entity.rs:17-20）。设计预留未实现。
- 【中等】**main.rs 该拆了**（1370 行、6 类关注点），建议按 Bevy plugin 切：
  1. `textures.rs`——`make_image` + 全部 `px_*`（main.rs:1158-1368，~210 行，最独立，先拆）
  2. `ui.rs`——`spawn_ui`/`draw_title`/`text_block`/`center_text`（941-1130）
  3. `fx.rs`——`particles`/`projectiles`/`spawn_fx`/`spawn_cast_fx`/`elem_burst`/`draw_beam` 等（591-938）
  4. `input.rs`——`handle_input`/`handle_normal`/`handle_aim`/`movement_dir`/`DIGIT_KEYS`
  5. `sync.rs`——`sync`/`actor_sprite`/`animate`/坐标映射
  6. `persistence.rs`——highscore IO（1132-1152）
- 【轻微】`Update` 全 `.chain()`（main.rs:70-73）：`particles`/`projectiles`/`camera_shake` 彼此独立无需串行（影响微乎其微）。
- 【优点】`level_gen` 串层防护正确：restart/descend 自增（main.rs:132, 308-310），sync 检测变化即 drain 所有 actor sprite（430-435）；逻辑层 descend 重置 entities 为 `vec![player]`（world.rs:347），配套无遗漏。

## 2. 正确性

- 【优点】边界安全扎实：`map.tile` 越界返回 `Wall`（map.rs:70-76）、`is_walkable` 越界 false、`is_opaque` 越界 true（map.rs:85-92）；`compute_fov` origin 越界有守卫（fov.rs:11）。无越界索引。
- 【优点】回合顺序正确：玩家死后不跑 `monster_turns`（world.rs:887-889），且每怪行动前重查玩家存活（924）；玩家 backfire 自杀路径正确收尾为 Dead（903-907）。
- 【优点】同格生成有防护（world.rs:373）；玩家起始房间不刷怪（369 `skip(1)`）。
- 【优点】施法边界完整：0 法力不 spend turn（308-316，有测试）；空词 `Err(Empty)`；超长词前后端双保险（main.rs:264 + runes.rs:320）。
- 【优点】**RNG 确定性实际很强**：玩家操作期间零 rng 消费（只有 `Map::generate`+`populate_level` 在 new/descend 时消费），给定 seed 每层布局完全固定；sim 内无 HashSet 迭代顺序依赖；前端粒子/shake 用非确定 `rand::rng()` 但不入 sim——分离正确。
- 【中等】**README 的 "determinism 有测试" 名不副实**：`determinism_same_seed_same_first_state`（playthrough.rs:41-47）只比首状态，没有多步回放一致性测试。
- 【优点】highscore IO 稳健：load 失败→0（main.rs:1143-1148），save 失败静默（1150-1152），HOME 缺失有 fallback（1136-1141），死亡只记一次（`recorded` flag，416）。
- 【中等】descend 状态清理基本完整但**漏清 `fx`/`cast_fx`**（world.rs:323-361）：当前无 bug（descend 路径不产生 fx 且上一步已被 drain），属防御性缺失——未来在 descend 路径 push fx 会串层。
- 【轻微】`apply_regen`（world.rs:890）在玩家死亡当回合仍执行一次。游戏已结束，无害。
- 【轻微·UX】法力不足施法会清空已编好的词：cast 后无条件 `composing.clear()`（main.rs:290, 336），但 `player_cast` 因 mana 不足只 log 不 spend——玩家词被清却没施成。

## 3. 代码质量

- 【轻微】clippy 唯一命中：playthrough.rs:16 `0 | 1 | 2` → `0..=2`。
- **新增 Rune variant 的完备性——三处点名**：
  - 【优点】`compile`（runes.rs:333-350）：match 全覆盖无 `_`，新增 variant 编译错误，安全。
  - 【优点】`apply_spell_to`（world.rs:789-803）：match 全覆盖，安全。
  - 【中等·H1】`rune_drop_pool`（world.rs:1006-1031）：**硬编码 Vec，非 match**——新增 Rune 静默不进掉落池，玩家永远拿不到。最实际的遗漏陷阱。
- 【中等·H2】`Rune::ALL: [Rune;16]`（runes.rs:51）、`Relic::ALL: [Relic;7]`（world.rs:43）手动数组：新增 variant 不加数组元素时编译通过、静默遗漏。可考虑 `strum::EnumIter` 或编译期断言。
- 【中等·H3】`is_damaging`/`contributes_power`（runes.rs:167,180）用 `matches!` 列举：新增 element 默认落入"非伤害/不贡献 power"，静默。
- 【中等·H4】**Relic 效果值双写**：`apply_relic` 实际数值（world.rs:482-498）与 `effect_desc` 展示字符串（world.rs:65-75）分离维护，改一处忘另一处 → UI 与实际不符。
- 【中等】**怪物表硬编码在 if-else**（world.rs:407-419）：名/字形/hp/power/regen/depth 权重全塞条件链，无集中平衡表。
- 【中等】magic number 集中度中等：核心值已抽常量（runes.rs:309-313；world.rs:15-21），但仍散落：`populate_level` 的怪数/掉落/relic 概率（world.rs:370,382-383,392）、`apply_spell_to` 的 burn+=3/slow+=2/Force 公式（794-801）、玩家初始三围（176-180）。
- 【轻微·H5】`Map::generate` 依赖"首次迭代必建房"的隐式不变式保 `rooms[0]` 非空（map.rs:109-137）。当前安全；若改小 MAP_WIDTH/调大 MAX_SIZE 可能 panic。加 `debug_assert!` 兜底。
- 【轻微】main.rs 中 Bevy `Entity` 与 `rune_core::Entity` 并存（567）、`ActorViews.map: HashMap<EntityId, Entity>`（154）命名易混但可接受。

## 4. 测试覆盖

**现盖（32 个）**：语法编译各分支 13 个、world 战斗/效果/边界 16 个、map determinism、400 步不变式 + 首状态 determinism。

**最值得补的 5 个**：
1. 【P0】**确定性多步回放**：同 seed 两个 World 跑相同操作序列 N 步，逐步断言全状态相同。README 声称却缺失的核心保证。
2. 【P0】**跨层状态清理**：descend 后断言 hp/mana 保留、burning/slowed 归零、rune_items/temp_walls 清空、entities 仅剩玩家、explored 重置、player.id==0。
3. 【P1】**战斗数值回归**：锁具体数值（Bolt+Fire → -4hp 且 burning==3；+Amplify → 7；Force → power/2+1），防平衡漂移（现测试多用 `hp < X` 方向断言）。
4. 【P0】**语法/组合边界（直接暴露已知 bug）**：Nova+Heal 是否治疗周围怪（B2）；Self+Frost 后玩家 slowed 是否生效（B1）；Self+Teleport 不 backfire；Touch+Barrier vs Nova+Barrier tile 数。
5. 【P1】**回合/边界**：地图边缘 Bolt 不 panic；Chain 打空是否意外索敌（S2）；玩家被烧死时 monster_turns 终止；死亡后各操作 no-op。

## 5. 性能

【结论：当前无真实风险，勿做微优化】
- 每步全量重建 static tile（sync:396-480）：60×36=2160 格只画 explored、仅在 `dirty` 时（回合制，非每帧）。Bevy spawn/despawn 几千 sprite 毫秒级，完全 OK。真要优化才上 dirty-rect。
- actor sprite 持久化 + lerp：正确做法，好设计。
- 粒子：每 fx 6-9 颗、beam 上限 40、全部 self-despawn，回合制节奏不累积。`std::mem::take` 零拷贝。
- 每帧系统遍历规模 <50，轻量。

## 6. 可扩展性

- **加新符文（元素类）要碰 ~13-15 处**：多数被 match 兜住（编译错误），但 `ALL` 数组 / `rune_drop_pool` / `is_damaging`(matches!) 是三个静默陷阱。
- **加新怪物**：`spawn_monster` if-else + `actor_sprite` glyph match，2 处，都硬编码。
- **数据驱动化——建议渐进而非全盘**：
  - 值得现在做：怪物表（if-else → `const [MonsterProto]` 含 depth 权重）、掉落权重（从 `Rune::ALL` 派生 + 权重表，杜绝 H1/H2）、Relic 数值+描述合一（消除 H4）。
  - **不建议数据化符文语法本身**：compile 的 match 完备性是核心安全价值，power/radius/range 是逻辑而非数据。
- **存档系统难度：中等，架构友好**。只需序列化 `World`，renderer 置 `dirty=true` 重建视图。工作量：~10+ 类型加 serde derive（机械）；唯一难点 `StdRng` 不可序列化——换 `rand_pcg`（支持 serde）或只存 seed+depth 在 descend 时重建 rng（玩家操作不消费 rng，rng 状态完全由 depth 决定，最省事）。估 1-2 天。

---

## 确认 Bug 列表

### B1【中等】玩家 `slowed` 状态永不生效（死字段）
- 位置：字段 entity.rs:37-38；施加 world.rs:795-796,810；玩家侧 `tick_status`（world.rs:953-985）只 tick burning；slowed 消费只在 `monster_turns`（927-930，仅对怪物）。
- 复现：`cast [OnSelf, Frost]`（Frost damaging → backfire 命中自己，slow+=2）。玩家 `slowed` 只增不减、从不丢回合。
- 修法：`end_turn` 玩家侧消费 slowed（跳过输入回合），或明确玩家免疫减速并删字段路径。

### B2【中等】Nova/Self+Heal 治疗敌人而非（或不含）自己
- 位置：Nova 分支 `tiles.retain(|&t| t != origin)`（world.rs:701）排除施法者；`ElementKind::Heal => healed += power` + `e.heal(healed)`（790,808）对任意 entity 生效，不分阵营。
- 复现：被围时 `cast [Nova, Heal]` → 周围怪全被治疗，玩家自己没回血（纯负面法术）。Beam/Bolt+Heal 同样奶怪。
- 修法：heal 按阵营过滤（只治施法者阵营），或明确"群疗"设计并让 Nova 含 origin。

### S2【轻微~中等·疑似】Chain 打空仍从施法者位置自动索敌
- 位置：world.rs:603-609，`hit` 为空时 `from=origin`（玩家位置），`nearest_unhit_enemy(origin, .., CHAIN_RANGE=4)`。
- 复现：`[Bolt, Lightning, Chain]` 瞄准无目标方向 → chain 仍从自身弧射 4 格内敌人。
- 修法：hit 为空时不启动 chain（`if let Some(last)=hit.last()`）。或明确为特性并驯化（限跳数/衰减）。

### S1【轻微·疑似】melee 击杀不触发吸血
- 位置：world.rs:522 `if atk_is_player && !died { … lifesteal }`——击杀时跳过 lifesteal（ignite 对死者无意义合理，lifesteal 在最该回血的场合失效）。
- 复现：装备 Leechstone，一击秒杀 → 不回血；未杀 → 回血。
- 修法：lifesteal 移出 `!died` 条件；若有意则注释固化。

---

## 改进优先级（工程视角）

### P0（马上做，低成本高价值）
- 修 B1（slowed 生效或移除）、B2（Heal 阵营过滤）。
- 补确定性多步回放测试 + 跨层清理测试 + 组合边界测试（覆盖 B1/B2/S2）。
- `rune_drop_pool` 改为从 `Rune::ALL` 派生 + 权重表（消 H1）；顺手修 clippy warning。

### P1（近期）
- 拆 main.rs：优先 textures.rs（最独立）→ ui.rs → fx.rs，再 input/sync/persistence，用 Bevy plugin 组织。
- 怪物表 / 掉落权重 / Relic 数值+描述 数据化（消 H4 双写），语法系统保持代码。
- 补战斗数值回归测试；给 `Fx` 加 `amount` 字段支持飘血 UI。
- 决定 S1/S2 是 bug 还是设计并注释固化。

### P2（远期/看路线图）
- 多速度怪/多重行动 → 能量累加器制回合引擎。
- 存档系统：World 加 serde derive + `rand_pcg`（或 seed+depth 重建 rng）。
- gameplay 调优：隔墙感知（world.rs:935）、Nova 排除自我的一致性、H2/H3 的 strum/编译期断言加固。
