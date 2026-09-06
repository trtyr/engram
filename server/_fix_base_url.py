p = 'crates/api/src/routes/llm_api.rs'
s = open(p, encoding='utf-8').read()

def rep(old, new):
    global s
    assert old in s, 'NOT FOUND: ' + repr(old[:70])
    s = s.replace(old, new, 1)

rep('''            Json(ProviderDto {
                id,
                name: req.name.trim().to_string(),
                base_url: req.base_url,''',
'''            Json(ProviderDto {
                id,
                name: req.name.trim().to_string(),
                base_url: base_url.clone(),''')

rep('''    // 校验提供的字段（与 create 同规则）
    if let Some(u) = &req.base_url {''',
'''    // 校验提供的字段（与 create 同规则；trim 粘贴空白）
    let base_url = req.base_url.as_ref().map(|u| u.trim().to_string());
    if let Some(u) = base_url.as_deref() {''')

rep('''        req.base_url.as_deref(),''', '''        base_url.as_deref(),''')

open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('update base_url patched')
