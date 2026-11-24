//! Raw-block constants: owning `const` declarations of raw `.air` blocks and collecting the
//! constants a tree's raw blocks use.

use super::*;

impl<'a> TreeLowerer<'a> {
    /// A raw `.air` constant belongs to the raw block that declares it, and its declaration ends
    /// in that block: its name (by full name, `K` or `T.e`) appears in no other raw block of the
    /// tree, in any role, and is declared in no other (a value several blocks share is a v2
    /// `const`). Each block is then analysed on its own, with the constants it declares and the
    /// v2 constants it names, exactly as the `.air` compiler reads it. Only a field (a word after
    /// `.`) is no name. The rule is lexical on purpose: a label, branch target or local named
    /// like another block's constant is rejected too, which a rename fixes.
    pub(super) fn own_raw_consts(
        &mut self,
        tokens: &[crate::lexer::Token],
        own_consts: &BTreeSet<String>,
        code: &str,
        span: Span,
    ) -> Result<()> {
        use crate::lexer::TokenKind;
        let key: RawBlockKey = (
            self.owner_module.clone(),
            span.line,
            span.column,
            code.to_owned(),
        );
        for (i, t) in tokens.iter().enumerate() {
            if matches!(&t.kind, TokenKind::Ident(kw) if kw == "const")
                && matches!(
                    tokens.get(i + 1).map(|t| &t.kind),
                    Some(TokenKind::Ident(_))
                )
                && !tokens[i..].iter().any(|t| t.kind == TokenKind::Semicolon)
            {
                return Err(Error::at(
                    span,
                    "a raw '.air' 'const' declaration must end with ';' in its own raw block",
                ));
            }
        }
        let mut names = BTreeSet::new();
        for (i, t) in tokens.iter().enumerate() {
            let TokenKind::Ident(word) = &t.kind else {
                continue;
            };
            let kind = |k: usize| tokens.get(i + k).map(|t| &t.kind);
            if i > 0 && tokens[i - 1].kind == TokenKind::Dot {
                continue;
            }
            match (kind(1), kind(2)) {
                (Some(TokenKind::Dot), Some(TokenKind::Ident(entry))) => {
                    names.insert(format!("{word}.{entry}"));
                }
                _ => {
                    names.insert(word.clone());
                }
            }
        }
        let shared = |name: &str| {
            Err(Error::at(
                span,
                format!(
                    "'{name}' is a raw '.air' constant of one raw block of this tree and appears \
                     in another; a raw constant's name appears only in its own block (share a \
                     value with a v2 'const', or rename)"
                ),
            ))
        };
        for name in own_consts {
            let other_owner = self.raw_const_owner.get(name).is_some_and(|k| *k != key);
            let other_user = self
                .raw_names
                .get(name)
                .is_some_and(|blocks| blocks.iter().any(|k| *k != key));
            if other_owner || other_user {
                return shared(name);
            }
            self.raw_const_owner.insert(name.clone(), key.clone());
        }
        for name in &names {
            if self.raw_const_owner.get(name).is_some_and(|k| *k != key) {
                return shared(name);
            }
            self.raw_names
                .entry(name.clone())
                .or_default()
                .insert(key.clone());
        }
        Ok(())
    }

    /// Records the v2 names a `raw { }` block uses as `.air` named constants of the tree, and
    /// returns the block's `const NAME = value;` declarations (for checking it on its own).
    ///
    /// A `const` name is its number (a GUID in hex) and `Table.entry` of a tuning table is its
    /// `bcon[N]` operand. The `.air` parser resolves them only where it takes an operand or an
    /// integer, so labels, branch targets, mode words, fields and argument keys keep their
    /// meaning, and a local the tree declares (`let x`, `local x`, tree-wide in `.air`) wins over
    /// a constant of the same name. [`Self::finish`] declares them at the top of the tree.
    ///
    /// One name must mean one value in a tree: an inlined helper's raw code may see another
    /// module's constants.
    pub(super) fn collect_raw_consts(&mut self, code: &str, span: Span) -> Result<String> {
        use crate::lexer::TokenKind;
        // Code that does not lex is reported by the `.air` compiler.
        let Ok(tokens) = crate::lexer::tokenize(code) else {
            return Ok(String::new());
        };
        // Discarded code (a dead branch) emits nothing, so it declares nothing in the tree.
        let live = self.checking_discarded == 0;
        // The block's own `.air` constants (`const K = ..;`, `const T.e = ..;`), by full name:
        // v2 declares none of them for this block. (A v2 constant of the same name that another
        // block uses is declared tree-wide, and `.air` rejects the clash unless the values agree.)
        let own_consts: BTreeSet<String> = tokens
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let kind = |k: usize| tokens.get(i + k).map(|t| &t.kind);
                if !matches!(&t.kind, TokenKind::Ident(kw) if kw == "const") {
                    return None;
                }
                let Some(TokenKind::Ident(name)) = kind(1) else {
                    return None;
                };
                match (kind(2), kind(3)) {
                    (Some(TokenKind::Dot), Some(TokenKind::Ident(entry))) => {
                        Some(format!("{name}.{entry}"))
                    }
                    _ => Some(name.clone()),
                }
            })
            .collect();
        self.own_raw_consts(&tokens, &own_consts, code, span)?;
        let mut found: Vec<(String, String)> = Vec::new();
        for (i, tok) in tokens.iter().enumerate() {
            let TokenKind::Ident(word) = &tok.kind else {
                continue;
            };
            if i > 0 && tokens[i - 1].kind == TokenKind::Dot {
                continue;
            }
            let next = |k: usize| tokens.get(i + k).map(|t| &t.kind);
            if matches!(word.as_str(), "let" | "local") {
                if live && let Some(TokenKind::Ident(name)) = next(1) {
                    self.raw_declared.insert(name.clone());
                }
                continue;
            }
            // An `.air` keyword (`temp`, `my`, ..) keeps its meaning: no constant can name it.
            if crate::parser::is_reserved_ident(word) {
                continue;
            }
            if next(1) == Some(&TokenKind::Dot)
                && let Some(TokenKind::Ident(entry)) = next(2)
                && own_consts.contains(&format!("{word}.{entry}"))
            {
                continue;
            }
            if next(1) == Some(&TokenKind::Dot)
                && let Some(TokenKind::Ident(entry)) = next(2)
                && let Some(entry_air) = self.env.lookup_bcon_entry(
                    &self.owner_module,
                    word,
                    entry,
                    self.in_object_scope,
                    span,
                )
            {
                found.push((format!("{word}.{entry}"), entry_air?.0));
                continue;
            }
            if own_consts.contains(word.as_str()) {
                continue;
            }
            if let Some(v) = self
                .env
                .lookup_const(&self.owner_module, word, self.in_object_scope)
            {
                let value = if i16::try_from(v).is_ok() {
                    v.to_string()
                } else {
                    format!("{v:#010x}")
                };
                found.push((word.clone(), value));
            }
        }
        let mut prelude = String::new();
        let mut declared = BTreeSet::new();
        for (name, value) in found {
            if !declared.insert(name.clone()) {
                continue;
            }
            // Discarded code emits nothing, so its constants neither register nor clash.
            match self.raw_consts.get(&name) {
                Some(old) if live && *old != value => {
                    return Err(Error::at(
                        span,
                        format!(
                            "'{name}' names {old} in one raw block of this tree and {value} in \
                             another; rename one of the constants"
                        ),
                    ));
                }
                Some(_) => {}
                None if live => {
                    self.raw_consts.insert(name.clone(), value.clone());
                }
                None => {}
            }
            prelude.push_str(&format!("const {name} = {value};\n"));
        }
        Ok(prelude)
    }
}
