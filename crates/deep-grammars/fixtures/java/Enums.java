package com.example.model;

public class Enums {
    public enum Status {
        ACTIVE(1) {
            @Override
            public boolean isOpen() {
                return true;
            }
        },
        CLOSED(0);

        private final int code;

        Status(int code) {
            this.code = code;
        }

        public boolean isOpen() {
            return false;
        }
    }

    public enum Kind {
        USER, ADMIN, GUEST
    }
}
