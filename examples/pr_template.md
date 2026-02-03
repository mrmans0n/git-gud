# PR Template Example

This is an example PR/MR template for git-gud (gg).

To use this template:
1. Copy this file to `.git/gg/pr_template.md` in your repository
2. Customize the template to fit your project's needs
3. When you run `gg sync`, the template will be used for all PR/MR descriptions

## Supported Placeholders

- `{{title}}` - The PR/MR title (from commit message first line)
- `{{description}}` - The commit description/body
- `{{stack_name}}` - Name of the current stack
- `{{commit_sha}}` - Short SHA of the commit

## Example Template

```markdown
## {{title}}

{{description}}

---

### Stack Information
- **Stack:** {{stack_name}}
- **Commit:** {{commit_sha}}

### Checklist
- [ ] Tests added/updated
- [ ] Documentation updated
- [ ] Ready for review
```

## Notes

- If the template file doesn't exist, gg uses its default behavior
- Templates only affect the PR/MR body, not the title
- Placeholders that aren't replaced remain as-is in the output
