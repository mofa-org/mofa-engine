from setuptools import setup

setup(
    name="mofa-sdk",
    version="0.1.0",
    description="MoFA Engine Python SDK — High Performance Intelligent Gateway Client",
    py_modules=["mofa_sdk"],
    install_requires=[
        "requests>=2.28.0",
    ],
)
