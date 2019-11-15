pub struct SessionList {
    sessions: Vec<crate::protocol::Session>,
    offset: usize,
    size: crate::term::Size,
}

impl SessionList {
    pub fn new(
        sessions: Vec<crate::protocol::Session>,
        size: crate::term::Size,
    ) -> Self {
        let mut by_name = std::collections::HashMap::new();
        for session in sessions {
            if !by_name.contains_key(&session.username) {
                by_name.insert(session.username.clone(), vec![]);
            }
            by_name.get_mut(&session.username).unwrap().push(session);
        }
        let mut names: Vec<_> = by_name.keys().cloned().collect();
        names.sort_by(|a: &String, b: &String| {
            let a_idle =
                by_name[a].iter().min_by_key(|session| session.idle_time);
            let b_idle =
                by_name[b].iter().min_by_key(|session| session.idle_time);
            // these unwraps are safe because we know that none of the vecs in
            // the map can be empty
            a_idle.unwrap().idle_time.cmp(&b_idle.unwrap().idle_time)
        });
        for name in &names {
            if let Some(sessions) = by_name.get_mut(name) {
                sessions.sort_by_key(|s| s.idle_time);
            }
        }

        let mut sorted = vec![];
        for name in names {
            let sessions = by_name.remove(&name).unwrap();
            for session in sessions {
                sorted.push(session);
            }
        }

        Self {
            sessions: sorted,
            offset: 0,
            size,
        }
    }

    pub fn visible_sessions(&self) -> &[crate::protocol::Session] {
        let start = self.offset;
        let end = self.offset + self.limit();
        let end = end.min(self.sessions.len());
        &self.sessions[start..end]
    }

    pub fn visible_sessions_with_chars(
        &self,
    ) -> impl Iterator<Item = (char, &crate::protocol::Session)> {
        self.visible_sessions()
            .iter()
            .enumerate()
            .map(move |(i, s)| (self.idx_to_char(i).unwrap(), s))
    }

    pub fn size(&self) -> crate::term::Size {
        self.size
    }

    pub fn resize(&mut self, size: crate::term::Size) {
        self.size = size;
    }

    pub fn id_for(&self, c: char) -> Option<&str> {
        self.char_to_idx(c).and_then(|i| {
            self.sessions.get(i + self.offset).map(|s| s.id.as_ref())
        })
    }

    pub fn next_page(&mut self) {
        let inc = self.limit();
        if self.offset + inc < self.sessions.len() {
            self.offset += inc;
        }
    }

    pub fn prev_page(&mut self) {
        let dec = self.limit();
        if self.offset >= dec {
            self.offset -= dec;
        }
    }

    pub fn current_page(&self) -> usize {
        self.offset / self.limit() + 1
    }

    pub fn total_pages(&self) -> usize {
        if self.sessions.is_empty() {
            1
        } else {
            (self.sessions.len() - 1) / self.limit() + 1
        }
    }

    fn idx_to_char(&self, mut i: usize) -> Option<char> {
        if i >= self.limit() {
            return None;
        }

        // 'q' shouldn't be a list option, since it is bound to quit
        if i >= 16 {
            i += 1;
        }

        #[allow(clippy::cast_possible_truncation)]
        Some(std::char::from_u32(('a' as u32) + (i as u32)).unwrap())
    }

    fn char_to_idx(&self, c: char) -> Option<usize> {
        if c == 'q' {
            return None;
        }

        let i = ((c as i32) - ('a' as i32)) as isize;
        if i < 0 {
            return None;
        }
        #[allow(clippy::cast_sign_loss)]
        let mut i = i as usize;

        // 'q' shouldn't be a list option, since it is bound to quit
        if i > 16 {
            i -= 1;
        }

        if i >= self.limit() {
            return None;
        }

        Some(i)
    }

    fn limit(&self) -> usize {
        let limit = self.size.rows as usize - 6;

        // enough for a-z except q - if we want to allow more than this, we'll
        // need to come up with a better way of choosing streams
        if limit > 25 {
            25
        } else {
            limit
        }
    }
}

#[cfg(test)]
#[allow(clippy::cognitive_complexity)]
#[allow(clippy::redundant_clone)]
#[allow(clippy::shadow_unrelated)]
mod test {
    use super::*;

    fn session(username: &str, idle: u32) -> crate::protocol::Session {
        crate::protocol::Session {
            id: format!("{}", uuid::Uuid::new_v4()),
            username: username.to_string(),
            term_type: "screen".to_string(),
            size: crate::term::Size { rows: 24, cols: 80 },
            idle_time: idle,
            title: "title".to_string(),
            watchers: 0,
        }
    }

    #[test]
    fn test_session_list_sorting() {
        let size = crate::term::Size { rows: 24, cols: 80 };

        let session1 = session("doy", 35);
        let session2 = session("doy", 3);
        let mut session3 = session("sartak", 12);
        let session4 = session("sartak", 100);
        let mut session5 = session("toft", 5);
        let mut sessions = vec![
            session1.clone(),
            session2.clone(),
            session3.clone(),
            session4.clone(),
            session5.clone(),
        ];

        assert_eq!(
            SessionList::new(sessions.clone(), size.clone()).sessions,
            vec![
                session2.clone(),
                session1.clone(),
                session5.clone(),
                session3.clone(),
                session4.clone(),
            ]
        );

        session3.idle_time = 2;
        sessions[2].idle_time = 2;
        assert_eq!(
            SessionList::new(sessions.clone(), size.clone()).sessions,
            vec![
                session3.clone(),
                session4.clone(),
                session2.clone(),
                session1.clone(),
                session5.clone(),
            ]
        );

        session5.idle_time = 1;
        sessions[4].idle_time = 1;
        assert_eq!(
            SessionList::new(sessions.clone(), size.clone()).sessions,
            vec![
                session5.clone(),
                session3.clone(),
                session4.clone(),
                session2.clone(),
                session1.clone(),
            ]
        );
    }

    #[test]
    fn test_session_list_pagination() {
        let size = crate::term::Size { rows: 11, cols: 80 };
        let sessions = vec![
            session("doy", 0),
            session("doy", 1),
            session("doy", 2),
            session("doy", 3),
            session("doy", 4),
            session("doy", 5),
            session("doy", 6),
            session("doy", 7),
            session("doy", 8),
            session("doy", 9),
            session("doy", 10),
        ];
        let mut list = SessionList::new(sessions.clone(), size);
        assert_eq!(list.limit(), 5);
        assert_eq!(list.total_pages(), 3);
        assert_eq!(list.current_page(), 1);

        list.next_page();
        assert_eq!(list.limit(), 5);
        assert_eq!(list.total_pages(), 3);
        assert_eq!(list.current_page(), 2);

        list.next_page();
        assert_eq!(list.limit(), 5);
        assert_eq!(list.total_pages(), 3);
        assert_eq!(list.current_page(), 3);

        list.next_page();
        assert_eq!(list.limit(), 5);
        assert_eq!(list.total_pages(), 3);
        assert_eq!(list.current_page(), 3);

        list.prev_page();
        assert_eq!(list.limit(), 5);
        assert_eq!(list.total_pages(), 3);
        assert_eq!(list.current_page(), 2);

        list.prev_page();
        assert_eq!(list.limit(), 5);
        assert_eq!(list.total_pages(), 3);
        assert_eq!(list.current_page(), 1);

        list.prev_page();
        assert_eq!(list.limit(), 5);
        assert_eq!(list.total_pages(), 3);
        assert_eq!(list.current_page(), 1);

        let id = list.id_for('a').unwrap();
        assert_eq!(id, sessions[0].id);
        let id = list.id_for('e').unwrap();
        assert_eq!(id, sessions[4].id);
        let id = list.id_for('f');
        assert!(id.is_none());

        list.next_page();
        let id = list.id_for('a').unwrap();
        assert_eq!(id, sessions[5].id);

        list.next_page();
        let id = list.id_for('a').unwrap();
        assert_eq!(id, sessions[10].id);
        let id = list.id_for('b');
        assert!(id.is_none());

        let size = crate::term::Size { rows: 24, cols: 80 };
        let sessions = vec![
            session("doy", 0),
            session("doy", 1),
            session("doy", 2),
            session("doy", 3),
            session("doy", 4),
            session("doy", 5),
            session("doy", 6),
            session("doy", 7),
            session("doy", 8),
            session("doy", 9),
            session("doy", 10),
            session("doy", 11),
            session("doy", 12),
            session("doy", 13),
            session("doy", 14),
            session("doy", 15),
            session("doy", 16),
            session("doy", 17),
            session("doy", 18),
            session("doy", 19),
            session("doy", 20),
            session("doy", 21),
        ];
        let list = SessionList::new(sessions.clone(), size);
        assert_eq!(list.limit(), 18);
        assert_eq!(list.total_pages(), 2);
        assert_eq!(list.current_page(), 1);

        let id = list.id_for('a').unwrap();
        assert_eq!(id, sessions[0].id);
        let id = list.id_for('p').unwrap();
        assert_eq!(id, sessions[15].id);
        let id = list.id_for('q');
        assert!(id.is_none());
        let id = list.id_for('r').unwrap();
        assert_eq!(id, sessions[16].id);
        let id = list.id_for('s').unwrap();
        assert_eq!(id, sessions[17].id);
        let id = list.id_for('t');
        assert!(id.is_none());
    }
}
