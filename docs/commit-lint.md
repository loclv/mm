# Commitlint

## Sample config file

A sample .commitlintrc.json configuration file uses @commitlint/config-conventional to enforce standard commit message rules. [1] (https://github.com/wppconnect-team/server-cli/blob/main/.commitlintrc.json)

```json
{
  "$schema": "https://json.schemastore.org/commitlintrc.json",
  "extends": ["@commitlint/config-conventional"],
  "rules": {
    "body-max-line-length": [0, "always", 128],
    "header-max-length": [2, "always", 128],
    "subject-case": [1, "never", ["sentence-case", "start-case", "pascal-case", "upper-case"]]
  }
}
```

## Explanation of Rules

- `$schema`: Points to the official JSON schema for validation and autocompletion in your editor.
- `extends`: Inherits preset rules from @commitlint/config-conventional.
- `body-max-line-length`: Sets the maximum character limit for body lines (level 0 disables enforcement here).
- `header-max-length`: Enforces a maximum length of 120 characters on the first line (header) with level 2 (error).
- `subject-case`: Warns (level 1) if the subject uses forbidden cases like PascalCase or UpperCase.

You can check full options and rule configuration details on the commitlint Configuration Reference. [1] (https://commitlint.js.org/reference/configuration.html)

## Sample Commit Messages

```text
# Conventional Commits format
feat: add new authentication module

BREAKING CHANGE: 
- Removed deprecated login API
- User session handling changed to JWT-based

# Conventional Commits format with scope
fix(api): resolve 500 error on user creation

- Fixed null validation error for email field
- Added proper error mapping for database exceptions

# Conventional Commits format with footer (breaking change)
perf(memory): optimize data fetching

- Reduced memory usage by 30%
- Implemented lazy loading for large datasets

BREAKING CHANGE: The data fetching strategy has been updated to use lazy loading.
Existing code relying on eager loading may need adjustments.
```
