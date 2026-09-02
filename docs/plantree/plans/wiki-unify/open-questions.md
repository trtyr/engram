# Open Questions

1. **/knowledge 兼容别名**：合并后 `/knowledge/*` 旧端点保留多久？选项：a) 保留一个过渡期（双注册，CLI/skill 慢慢迁）；b) 直接删（CLI/skill 同批迁完）。影响 memory.py 的 knowledge 命令 + 现有消费方。
2. **自动织触发时机**：上传文档后「自动织」是同步还是后台队列？倾向后台队列（织是 LLM 任务，同步会卡上传），但要不要给用户一个「立即织」按钮。
3. **织的粒度**：一个 document 织成一个 page，还是 document 里的多个主题织成多个 page？现状 wiki ingest 是「一段源文本 → 多个页面」。
4. **图谱默认视图**：合并后 Wiki 的默认 tab 是「文档」还是「图谱」？Obsidian 打开默认是文件列表，图谱是切换视图。
