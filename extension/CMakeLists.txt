cmake_minimum_required(VERSION 3.14)
project(rocklake_extension)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Find the rocklake shared library
find_library(ROCKLAKE_LIB rocklake_ffi
    PATHS ${CMAKE_CURRENT_SOURCE_DIR}/../target/release
          ${CMAKE_CURRENT_SOURCE_DIR}/../target/debug)

# Include the generated C header
include_directories(${CMAKE_CURRENT_SOURCE_DIR}/include)

# Extension source
add_library(rocklake_extension SHARED
    src/rocklake_extension.cpp
)

target_link_libraries(rocklake_extension ${ROCKLAKE_LIB})

set_target_properties(rocklake_extension PROPERTIES
    OUTPUT_NAME "rocklake"
    PREFIX ""
    SUFFIX ".duckdb_extension"
)
