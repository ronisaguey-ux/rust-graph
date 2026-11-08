const std = @import("std");

const Config = struct {
    depth: u32 = 12,
    follow_symlinks: bool = false,
};

pub fn main() !void {
    const allocator = std.heap.page_allocator;
    const args = try std.process.argsAlloc(allocator);
    defer std.process.argsFree(allocator, args);

    if (args.len < 2) {
        std.debug.print("usage: scanner <root>\n", .{});
        return;
    }

    const root = args[1];
    var config: Config = .{ .depth = 8, .follow_symlinks = false };
    const count = try scan(allocator, root, config);
    std.debug.print("{d} files\n", .{count});
}

fn scan(allocator: std.mem.Allocator, root: []const u8, config: Config) !usize {
    var dir = try std.fs.cwd().openDir(root, .{ .iterate = true });
    defer dir.close();

    var iterator = dir.iterate();
    var count: usize = 0;
    var next = try iterator.next();
    while (next != null) {
        count += 1;
        next = try iterator.next();
    }
    return count;
}
