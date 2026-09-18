//! 登录防爆破限速（公网加固，公网多Agent P001-t8）：
//! 单用户系统按用户名维度限速——连续失败 5 次锁 15 分钟，成功登录清零。
//! 进程内状态（重启即清零）；公网部署的纵深限速（连接层/反代层）另行配置，
//! 本层挡住最廉价的密码暴力枚举。
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 连续失败阈值——达到即进入锁定窗口
const MAX_FAILURES: u32 = 5;
/// 锁定窗口
const LOCK_WINDOW: Duration = Duration::from_secs(15 * 60);

struct Entry {
    fails: u32,
    locked_until: Option<Instant>,
}

fn registry() -> &'static Mutex<HashMap<String, Entry>> {
    static REG: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 单用户系统用户名只有 admin 一个，但 key 归一化保留扩展性（多账号不再改这里）
fn key_of(username: &str) -> String {
    username.trim().to_lowercase()
}

/// 登录前检查：该用户名是否处于锁定窗口（handler 转 429）。
/// 窗口已过则顺带解锁清零，重新计。
#[must_use]
pub fn is_locked(username: &str) -> bool {
    let key = key_of(username);
    let mut reg = registry().lock().unwrap();
    let Some(e) = reg.get_mut(&key) else {
        return false;
    };
    let Some(until) = e.locked_until else {
        return false;
    };
    if Instant::now() < until {
        return true;
    }
    // 锁定窗口已过：重新计
    e.locked_until = None;
    e.fails = 0;
    false
}

/// 登录失败：计数 +1，达阈值进锁定窗口。
pub fn record_failure(username: &str) {
    let key = key_of(username);
    let mut reg = registry().lock().unwrap();
    let e = reg.entry(key).or_insert(Entry {
        fails: 0,
        locked_until: None,
    });
    e.fails = e.fails.saturating_add(1);
    if e.fails >= MAX_FAILURES {
        e.locked_until = Some(Instant::now() + LOCK_WINDOW);
    }
}

/// 登录成功：清除该用户名的全部失败记录。
pub fn record_success(username: &str) {
    registry().lock().unwrap().remove(&key_of(username));
}

/// 测试钩子：清空限速表（集成测试用独特用户名隔离，必要时整体复位）。
pub fn clear_all() {
    registry().lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_after_max_failures_and_recovers() {
        let user = "unit-lock-user";
        for _ in 0..(MAX_FAILURES - 1) {
            record_failure(user);
        }
        assert!(!is_locked(user), "阈值前不锁定");
        record_failure(user); // 第 MAX_FAILURES 次 → 锁定
        assert!(is_locked(user), "达阈值后锁定");
        record_success(user);
        assert!(!is_locked(user), "成功登录即清零");
    }

    #[test]
    fn username_case_and_whitespace_normalized() {
        // 独立用户名——限速表是进程级全局，libtest 并行跑测试，禁止用 clear_all 交叉复位
        let user = "unit-case-user";
        for _ in 0..MAX_FAILURES {
            record_failure(user);
        }
        assert!(is_locked(user), "本名应锁定");
        assert!(
            is_locked(" UNIT-CASE-USER "),
            "大小写/空白归一后应命中同一条目"
        );
    }
}
