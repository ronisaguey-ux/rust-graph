package com.example;

import java.util.List;
import java.util.Map.Entry;

public class Store extends Base implements Repo {
    private final List<String> names;

    public Store(List<String> names) {
        this.names = names;
    }

    public String find(int id) {
        String name = this.names.get(id);
        Logger.log(name.trim());
        return name;
    }
}
