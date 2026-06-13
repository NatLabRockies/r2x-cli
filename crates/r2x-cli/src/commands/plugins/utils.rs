/// Shorten a git commit hash to 7 characters.
pub fn short_commit(commit: &str) -> &str {
    const SHORT_COMMIT_LEN: usize = 7;
    if commit.len() > SHORT_COMMIT_LEN {
        &commit[..SHORT_COMMIT_LEN]
    } else {
        commit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_commit_uses_seven_char_prefix() {
        assert_eq!(short_commit("abcdef012345"), "abcdef0");
        assert_eq!(short_commit("abc123"), "abc123");
    }
}
