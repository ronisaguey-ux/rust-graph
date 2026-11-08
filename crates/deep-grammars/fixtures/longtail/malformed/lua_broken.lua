local function walk(root
    for path in walk(root) do
        print(path)
    end
end

return walk
