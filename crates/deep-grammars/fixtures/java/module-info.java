open module com.example.app {
    requires transitive java.base;
    requires static java.logging;
    exports com.example.api;
    exports com.example.internal to com.example.tests;
    opens com.example.model;
    uses com.example.spi.Plugin;
    provides com.example.spi.Plugin with com.example.plugins.DefaultPlugin;
}
