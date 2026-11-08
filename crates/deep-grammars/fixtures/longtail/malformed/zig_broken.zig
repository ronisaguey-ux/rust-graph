const std = @import("std");

pub fn scan(root: []const u8) !usize {
    var dir = try std.fs.cwd().openDir(root, .{});
    defer dir.close();
    return 0;
