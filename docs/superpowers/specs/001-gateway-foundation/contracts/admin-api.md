# Contract: Admin API

**Feature**: 001-gateway-foundation | 承载方式：Pingora `ServeHttp`，独立 admin listener（见 [research.md](../research.md) R7）

机器可读管理接口（FR-023）。**每个**请求经 `Principal` / `AuthContext` / `authorize(action, resource)` 边界鉴权，bootstrap 模式亦然（FR-024/FR-025）。响应体为 JSON。缺失/无效凭据一律 `401`/`403` 且不泄露内部细节（SC-006）。字段名英文（宪法 XXII）。

## 鉴权

- 凭据：bootstrap 静态管理员凭据（如 `Authorization: Bearer <admin-token>`），但**不得**以全局等值判断实现——经 `authorize(action, resource)`（FR-025）。
- 每个端点声明所需 `action` + `resource`，便于未来映射 OAuth/OIDC/角色（仅预留边界）。

## 端点

### `GET /healthz` — 存活
- **action/resource**: `admin.health.read` / `gateway`
- **200**：`{ "status": "ok" }`
- 用途：进程存活探针。

### `GET /readyz` — 就绪
- **action/resource**: `admin.ready.read` / `gateway`
- **200**：`{ "status": "ready", "active_version": "<version>" }`
- **503**：`{ "status": "not_ready", "reason": "<...>" }`（无有效 snapshot 时）
- 用途：流量就绪探针 + 当前生效配置版本（SC-007）。

### `POST /config/validate` — 配置校验
- **action/resource**: `admin.config.validate` / `config`
- **请求**：候选 `pingogate.yaml` 内容（`text/yaml` 或 `{ "config": "<yaml>" }`）
- **200**：`{ "valid": true }`
- **422**：`{ "valid": false, "errors": [ { "path": "...", "message": "..." } ] }`
- **不变式**：MUST NOT 改变活跃运行时（FR-026）。

### `POST /reload` — 触发重载
- **action/resource**: `admin.config.reload` / `config`
- **请求**：无 body（重载磁盘 `pingogate.yaml`）或携带候选配置
- **200**：`{ "result": "success", "active_version": "<v+1>", "rollback_target": "<v>" }`
- **422**：`{ "result": "rejected", "errors": [...] }`（校验失败，活跃 snapshot 不变，FR-020）
- **行为**：执行 FR-019 重载流程；与 SIGHUP/文件监听同一路径。

### `GET /reload/status` — 重载状态
- **action/resource**: `admin.config.read` / `config`
- **200**：
  ```json
  {
    "last_result": "success | rejected",
    "active_version": "<version>",
    "rollback_target": "<version | null>",
    "timestamp": "<RFC3339>",
    "errors": []
  }
  ```
- 用途：查询最近一次重载结果、生效版本与回滚目标（FR-027）。

## 契约测试（宪法 XIII，先写并失败）

1. 无凭据/错误凭据访问任一端点 → 401/403，且未触达受保护操作（SC-006）。
2. 有效凭据 `GET /healthz`、`/readyz` → 结构化存活/就绪 + 版本。
3. `POST /config/validate` 合法配置 → `{valid:true}`；非法配置 → `{valid:false, errors:[...]}` 且活跃运行时不变。
4. `POST /reload` 合法 → success + 新版本 + 回滚目标；非法 → rejected 且 `GET /reload/status` 反映旧版本仍生效。
5. 鉴权全程经 `authorize` 边界（断言无全局 token 等值分支，FR-025）。
