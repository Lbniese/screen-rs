/// Build a TERMCAP environment string for the PTY child process.
///
/// The TERMCAP entry describes the terminal capabilities to applications
/// running inside the window.  Base capabilities come from the xterm-256color
/// entry; user overrides (from `termcap` / `terminfo` config lines) can
/// add or replace individual capabilities.
/// Build the full TERMCAP string from the base terminal description
/// merged with user-supplied overrides.
pub(crate) fn build_termcap(
    term: &str,
    columns: u16,
    rows: u16,
    overrides: &[(Vec<u8>, Vec<u8>)],
) -> Vec<u8> {
    // Start with a minimal base entry: screen-256color
    let mut caps = base_termcap(term, columns, rows);

    // Apply per-term overrides: replace matching capabilities
    for (ov_term, ov_caps) in overrides {
        let ov_name = String::from_utf8_lossy(ov_term);
        if ov_name == term || ov_name == "*" {
            let ov_str = String::from_utf8_lossy(ov_caps);
            merge_overrides(&mut caps, &ov_str);
        }
    }

    caps.into_bytes()
}

/// Base termcap entry resembling xterm-256color / screen-256color.
/// All values use colon separators as traditional termcap expects.
fn base_termcap(term: &str, cols: u16, rows: u16) -> String {
    // Build a minimal but functional entry
    let mut s = String::with_capacity(1024);

    // Terminal names (termcap format: first entry is name list separated by |)
    let name = format!("{term}|screen-rs|vt100|xterm");
    s.push_str(&name);

    // Boolean capabilities
    s.push_str(":am"); // auto-margin (wrap at right margin)
    s.push_str(":km"); // meta key sends escape
    s.push_str(":mi"); // safe to move in insert mode
    s.push_str(":ms"); // safe to move in standout mode
    s.push_str(":xn"); // newline ignored after 80 cols
    s.push_str(":AX"); // terminal uses xterm SGR mouse mode
    s.push_str(":G0"); // terminal can use G0 character set
    s.push_str(":XT"); // xterm-like terminal
    s.push_str(":bw"); // backward auto-margin
    s.push_str(":NP"); // no ESC in padding

    // Numeric capabilities
    s.push_str(&format!(":co#{cols}")); // columns
    s.push_str(&format!(":li#{rows}")); // lines
    s.push_str(":it#8"); // tabs every 8 spaces
    s.push_str(":vt#3"); // virtual terminal
    s.push_str(":Co#256"); // 256 colors
    s.push_str(":pa#32767"); // max parameters

    // String capabilities (common ones)
    s.push_str(":bl=\\Eg"); // bell (visible bell)
    s.push_str(":cr=\\r"); // carriage return
    s.push_str(":csr=\\E[%i%p1%d;%p2%dr"); // change scroll region
    s.push_str(":cub=\\E[%p1%dD"); // cursor backward
    s.push_str(":cub1=^H"); // cursor left 1
    s.push_str(":cud=\\E[%p1%dB"); // cursor down
    s.push_str(":cud1=\\n"); // cursor down 1
    s.push_str(":cuf=\\E[%p1%dC"); // cursor forward
    s.push_str(":cuf1=\\E[C"); // cursor right 1
    s.push_str(":cup=\\E[%i%p1%d;%p2%dH"); // cursor position
    s.push_str(":cuu=\\E[%p1%dA"); // cursor up
    s.push_str(":cuu1=\\E[A"); // cursor up 1
    s.push_str(":clear=\\E[H\\E[J"); // clear screen
    s.push_str(":ed=\\E[J"); // clear to end of display
    s.push_str(":el=\\E[K"); // clear to end of line
    s.push_str(":el1=\\E[1K"); // clear to start of line
    s.push_str(":home=\\E[H"); // cursor home
    s.push_str(":hpa=\\E[%p1%dG"); // horizontal position absolute
    s.push_str(":vpa=\\E[%p1%dd"); // vertical position absolute
    s.push_str(":ich=\\E[%p1%d@"); // insert characters
    s.push_str(":ich1=\\E[@"); // insert character
    s.push_str(":dch=\\E[%p1%dP"); // delete characters
    s.push_str(":dch1=\\E[P"); // delete character
    s.push_str(":il=\\E[%p1%dL"); // insert lines
    s.push_str(":il1=\\E[L"); // insert line
    s.push_str(":dl=\\E[%p1%dM"); // delete lines
    s.push_str(":dl1=\\E[M"); // delete line
    s.push_str(":ind=\\n"); // scroll forward
    s.push_str(":ri=\\EM"); // scroll reverse
    s.push_str(":rin=\\E[%p1%dT"); // scroll reverse N lines
    s.push_str(":indn=\\E[%p1%dS"); // scroll forward N lines

    // Key definitions (for function keys)
    s.push_str(":k1=\\EOP"); // F1
    s.push_str(":k2=\\EOQ"); // F2
    s.push_str(":k3=\\EOR"); // F3
    s.push_str(":k4=\\EOS"); // F4
    s.push_str(":k5=\\E[15~"); // F5
    s.push_str(":k6=\\E[17~"); // F6
    s.push_str(":k7=\\E[18~"); // F7
    s.push_str(":k8=\\E[19~"); // F8
    s.push_str(":k9=\\E[20~"); // F9
    s.push_str(":k;=\\E[21~"); // F10
    s.push_str(":F1=\\E[23~"); // F11
    s.push_str(":F2=\\E[24~"); // F12
    s.push_str(":kh=\\E[H"); // home key
    s.push_str(":@7=\\E[F"); // end key
    s.push_str(":kI=\\E[2~"); // insert key
    s.push_str(":kD=\\E[3~"); // delete key
    s.push_str(":kP=\\E[5~"); // page up
    s.push_str(":kN=\\E[6~"); // page down
    s.push_str(":ku=\\EA"); // cursor up
    s.push_str(":kd=\\EB"); // cursor down
    s.push_str(":kl=\\ED"); // cursor left
    s.push_str(":kr=\\EC"); // cursor right

    // Visual attributes
    s.push_str(":md=\\E[1m"); // bold
    s.push_str(":me=\\E[0m"); // normal
    s.push_str(":mr=\\E[7m"); // reverse video
    s.push_str(":us=\\E[4m"); // underline
    s.push_str(":ue=\\E[24m"); // underline off
    s.push_str(":so=\\E[7m"); // standout
    s.push_str(":se=\\E[27m"); // standout off
    s.push_str(":mb=\\E[5m"); // blink

    // Colors (simplified - full 256-color via setab/setaf)
    s.push_str(":AB=\\E[4%dm"); // ANSI set background color
    s.push_str(":AF=\\E[3%dm"); // ANSI set foreground color
    s.push_str(":op=\\E[39;49m"); // original color pair

    // Misc
    s.push_str(":ac=``aaffggjjkkllmmnnooqqssttuuvvwwxx~~");
    s.push_str(":ae=\\E(B"); // end alternate charset
    s.push_str(":as=\\E(0"); // start alternate charset
    s.push_str(":eA=\\E(B\\E)0"); // enable all charsets
    s.push_str(":kbs=^?"); // backspace
    s.push_str(":sc=\\E7"); // save cursor
    s.push_str(":rc=\\E8"); // restore cursor
    s.push_str(":ti=\\E[?1049h"); // enter ca mode
    s.push_str(":te=\\E[?1049l"); // exit ca mode
    s.push_str(":u6=\\E[%i%d;%dR"); // cursor position report
    s.push_str(":u7=\\E[6n"); // cursor position request
    s.push_str(":u8=\\E[?%[;0123456789]c"); // DA response
    s.push_str(":u9=\\E[c"); // DA request

    s
}

/// Merge override capabilities into the base termcap string.
/// Each override is a colon-separated capability that replaces or
/// adds to the base entry.
fn merge_overrides(base: &mut String, overrides: &str) {
    for cap in overrides.split(':') {
        let cap = cap.trim();
        if cap.is_empty() {
            continue;
        }

        // Extract the capability name (up to first # or =)
        let cap_name = match cap.find(['#', '=']) {
            Some(pos) => &cap[..pos],
            None => cap,
        };

        // Remove existing entry for this capability
        remove_capability(base, cap_name);

        // Append the new capability
        if !base.ends_with(':') {
            base.push(':');
        }
        base.push_str(cap);
    }
}

/// Remove a specific capability from the termcap string.
fn remove_capability(entry: &mut String, name: &str) {
    // Capabilities are colon-separated. Find and remove the one matching `name`.
    let mut new = String::with_capacity(entry.len());
    for segment in entry.split(':') {
        if segment.is_empty() {
            continue;
        }
        let seg_name = match segment.find(['#', '=']) {
            Some(pos) => &segment[..pos],
            None => segment,
        };
        if seg_name != name {
            if !new.is_empty() {
                new.push(':');
            }
            new.push_str(segment);
        }
    }
    *entry = new;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_termcap_has_dimensions() {
        let tc = base_termcap("xterm", 80, 24);
        assert!(tc.contains(":co#80:"));
        assert!(tc.contains(":li#24:"));
    }

    #[test]
    fn overrides_replace_capabilities() {
        let mut caps = base_termcap("xterm", 80, 24);
        // Override columns
        merge_overrides(&mut caps, "co#132:am:");
        assert!(caps.contains(":co#132:"));
        assert!(!caps.contains(":co#80:"));
    }

    #[test]
    fn overrides_add_new_capabilities() {
        let mut caps = base_termcap("xterm", 80, 24);
        merge_overrides(&mut caps, "ZZ=test");
        assert!(caps.contains(":ZZ=test"));
    }

    #[test]
    fn build_termcap_with_matching_term() {
        let overrides = vec![(b"xterm".to_vec(), b"co#132".to_vec())];
        let tc = build_termcap("xterm", 80, 24, &overrides);
        let tc_str = String::from_utf8_lossy(&tc);
        assert!(tc_str.contains("co#132"));
        assert!(!tc_str.contains("co#80"));
    }

    #[test]
    fn build_termcap_wildcard_override() {
        let overrides = vec![(b"*".to_vec(), b"li#60".to_vec())];
        let tc = build_termcap("screen", 80, 24, &overrides);
        let tc_str = String::from_utf8_lossy(&tc);
        assert!(tc_str.contains("li#60"));
        assert!(!tc_str.contains("li#24"));
    }

    #[test]
    fn build_termcap_non_matching_term_ignored() {
        let overrides = vec![(b"vt220".to_vec(), b"co#40".to_vec())];
        let tc = build_termcap("xterm", 80, 24, &overrides);
        let tc_str = String::from_utf8_lossy(&tc);
        assert!(tc_str.contains(":co#80:")); // not overridden
    }
}
