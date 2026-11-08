package broken;

public class Broken {
    private final String message = "this never closes;

    public String message() {
        return message;
    }
}
