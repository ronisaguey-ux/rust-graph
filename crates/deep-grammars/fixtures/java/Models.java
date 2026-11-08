package com.example.model;

import java.time.Instant;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;

public class Models {
    public record User(long id, String name, List<String> tags, Instant created) {
        public User {
            Objects.requireNonNull(name);
            tags = List.copyOf(tags);
        }

        public User withName(String next) {
            return new User(id, next, tags, created);
        }
    }

    public static final class Audit {
        private final long userId;
        private final String action;
        private final List<String> notes = new ArrayList<>();

        public Audit(long userId, String action) {
            this.userId = userId;
            this.action = action;
        }

        public Audit note(String note) {
            notes.add(note);
            return this;
        }

        public int noteCount() {
            return notes.size();
        }
    }
}
