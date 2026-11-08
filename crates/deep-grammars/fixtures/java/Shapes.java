package com.example.geometry;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Function;

public class Shapes {
    public static List<Double> areas(List<Shape> shapes) {
        List<Double> out = new ArrayList<>();
        for (Shape shape : shapes) {
            out.add(shape.area());
        }
        return out;
    }

    public static Shape transform(Shape shape, Function<Shape, Shape> f) {
        return f.apply(shape);
    }

    public static double total(Shape... shapes) {
        double sum = 0.0;
        for (int i = 0; i < shapes.length; i++) {
            sum += shapes[i].area();
        }
        return sum;
    }

    public static Runnable printer(String label) {
        return () -> System.out.println(label);
    }
}
