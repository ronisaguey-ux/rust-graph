plugins {
    kotlin("jvm") version "1.9.24"
    application
}

repositories {
    mavenCentral()
}

dependencies {
    implementation(kotlin("stdlib"))
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.2")
}

kotlin {
    jvmToolchain(21)
}

application {
    mainClass.set("com.example.MainKt")
}

tasks.test {
    useJUnitPlatform()
}

val sources = fileTree("src/main/kotlin") {
    include("**/*.kt")
}

tasks.register("count") {
    doLast {
        println("${sources.files.size} files")
    }
}
