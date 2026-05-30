import java.io.File

plugins {
    id("java-library")
    kotlin("jvm") version "1.9.0"
    `maven-publish`
    signing
}

group = "io.trickle"
version = "0.44.0"

java {
    sourceCompatibility = JavaVersion.VERSION_21
    targetCompatibility = JavaVersion.VERSION_21
}

kotlin {
    jvmToolchain(21)
}

repositories {
    mavenCentral()
}

dependencies {
    // Core Java dependencies
    implementation("org.slf4j:slf4j-api:2.0.11")
    
    // Kotlin coroutines
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.7.3")
    
    // Testing
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.1")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.7.3")
}

tasks.register<Exec>("buildNativeLibrary") {
    description = "Build the native RockLake library using cargo"
    group = "build"
    
    val cargoDir = rootProject.projectDir.parentFile
    workingDir = cargoDir
    
    commandLine("cargo", "build", "--release", "-p", "rocklake-ffi")
    
    doLast {
        // Copy native libraries to resources
        val nativeDir = File(buildDir, "resources/main/native")
        nativeDir.mkdirs()
        
        val releaseDir = File(cargoDir, "target/release")
        listOf(
            "librocklake.so" to "rocklake-linux-x86_64.so",
            "librocklake.dylib" to "rocklake-macos-arm64.dylib",
            "rocklake.dll" to "rocklake-windows-x86_64.dll"
        ).forEach { (src, dst) ->
            val srcFile = File(releaseDir, src)
            if (srcFile.exists()) {
                srcFile.copyTo(File(nativeDir, dst), overwrite = true)
            }
        }
    }
}

tasks.getByName("processResources").dependsOn("buildNativeLibrary")

java {
    withSourcesJar()
    withJavadocJar()
}

publishing {
    publications {
        create<MavenPublication>("mavenJava") {
            from(components["java"])
            
            pom {
                name.set("RockLake Java Bindings")
                description.set("JVM bindings for the RockLake serverless lakehouse catalog")
                url.set("https://github.com/trickle-labs/rocklake")
                
                licenses {
                    license {
                        name.set("Apache License 2.0")
                        url.set("https://www.apache.org/licenses/LICENSE-2.0.txt")
                    }
                }
                
                developers {
                    developer {
                        id.set("trickle")
                        name.set("Trickle Labs")
                        email.set("dev@trickle.so")
                    }
                }
                
                scm {
                    connection.set("scm:git:https://github.com/trickle-labs/rocklake.git")
                    developerConnection.set("scm:git:ssh://git@github.com/trickle-labs/rocklake.git")
                    url.set("https://github.com/trickle-labs/rocklake")
                }
            }
        }
    }
    
    repositories {
        maven {
            name = "GitHubPackages"
            url = uri("https://maven.pkg.github.com/trickle-labs/rocklake")
            credentials {
                username = System.getenv("GITHUB_ACTOR")
                password = System.getenv("GITHUB_TOKEN")
            }
        }
    }
}

signing {
    sign(publishing.publications["mavenJava"])
}

tasks.test {
    useJUnitPlatform()
}
