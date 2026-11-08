function scan(root::String
    files = readdir(root)
    for file in files
        println(file)
    end
end
