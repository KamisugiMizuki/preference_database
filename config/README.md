# config/

本地配置文件目录。此目录下所有文件（含登录凭证）**不纳入版本控制**（见根目录 `.gitignore`）。

## bangumi_cookie.txt

Bangumi 源的登录 Cookie，可选配置。获取与配置方法见项目根目录 `README.md` 的「Bangumi Cookie 配置」一节。

- 文件不存在 → Bangumi 源以匿名方式搜索（不显示 R18）
- 文件存在 → 搜索请求携带 Cookie；请求失败自动回退匿名搜索

**注意**：此文件包含你的账号登录凭证，切勿分享、切勿提交到 Git。
