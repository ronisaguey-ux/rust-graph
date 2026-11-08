package com.example.store;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

public class Repository<T> {
    private final Map<Long, T> rows = new LinkedHashMap<>();
    private long nextId = 1L;

    public long insert(T row) {
        long id = nextId++;
        rows.put(id, row);
        return id;
    }

    public Optional<T> find(long id) {
        return Optional.ofNullable(rows.get(id));
    }

    public List<T> all(int limit) {
        List<T> out = new ArrayList<>();
        for (T row : rows.values()) {
            if (out.size() >= limit) {
                break;
            }
            out.add(row);
        }
        return out;
    }

    public int delete(long id) {
        if (rows.remove(id) == null) {
            throw new IllegalArgumentException("missing " + id);
        }
        return rows.size();
    }
}
