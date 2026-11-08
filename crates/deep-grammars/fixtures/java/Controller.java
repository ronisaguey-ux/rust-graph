package com.example.web;

import java.util.List;
import java.util.Map;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class Controller {
    private final Map<String, List<String>> index;

    public Controller(Map<String, List<String>> index) {
        this.index = index;
    }

    @GetMapping("/items")
    public List<String> items(String q) {
        List<String> values = index.getOrDefault(q, List.of());
        for (String value : values) {
            if (value == null || value.isBlank()) {
                continue;
            }
            System.out.println(value);
        }
        return values;
    }

    public int count() {
        int total = 0;
        for (Map.Entry<String, List<String>> entry : index.entrySet()) {
            total += entry.getValue().size();
        }
        return total;
    }
}
