const std = @import("std");

pub const Edge = struct {
    from: u32,
    to: u32,
    weight: f64,
};

pub const Graph = struct {
    edges: std.ArrayList(Edge),
    nodes: usize,

    pub fn init(allocator: std.mem.Allocator, nodes: usize) Graph {
        return .{
            .edges = std.ArrayList(Edge).init(allocator),
            .nodes = nodes,
        };
    }

    pub fn deinit(self: *Graph) void {
        self.edges.deinit();
    }

    pub fn add(self: *Graph, from: u32, to: u32, weight: f64) !void {
        try self.edges.append(.{ .from = from, .to = to, .weight = weight });
    }

    pub fn total(self: Graph) f64 {
        var sum: f64 = 0;
        var i: usize = 0;
        while (i < self.edges.items.len) : (i += 1) {
            sum += self.edges.items[i].weight;
        }
        return sum;
    }
};
