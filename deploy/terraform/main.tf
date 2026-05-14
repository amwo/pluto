terraform {
  required_version = ">= 1.6"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
}

provider "aws" {
  region = var.region
}

variable "region" {
  type    = string
  default = "eu-west-2"
}

variable "az" {
  type    = string
  default = "eu-west-2a"
}

variable "instance_type" {
  type    = string
  default = "c7i.large"
}

variable "operator_ssh_key" {
  description = "SSH public key (root@pluto) — deploy-rs uses this to push generations"
  type        = string
}

variable "operator_cidr" {
  description = "CIDR allowed to SSH (default 0.0.0.0/0; restrict to your IP if static)"
  type        = string
  default     = "0.0.0.0/0"
}

variable "nixos_ami" {
  description = "NixOS 24.11 AMI in eu-west-2 (override per region/version)"
  type        = string
  default     = "ami-0c0c5fa9e0c4d5d50"
}

data "aws_vpc" "default" {
  default = true
}

data "aws_subnets" "default_az" {
  filter {
    name   = "vpc-id"
    values = [data.aws_vpc.default.id]
  }
  filter {
    name   = "availability-zone"
    values = [var.az]
  }
}

resource "aws_security_group" "pluto" {
  name        = "pluto"
  description = "pluto: ssh ingress for deploy-rs + egress all"
  vpc_id      = data.aws_vpc.default.id

  ingress {
    from_port   = 22
    to_port     = 22
    protocol    = "tcp"
    cidr_blocks = [var.operator_cidr]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

resource "aws_instance" "pluto" {
  ami                    = var.nixos_ami
  instance_type          = var.instance_type
  subnet_id              = data.aws_subnets.default_az.ids[0]
  vpc_security_group_ids = [aws_security_group.pluto.id]

  user_data = <<-EOT
    #!/usr/bin/env bash
    mkdir -p /root/.ssh
    chmod 700 /root/.ssh
    echo "${var.operator_ssh_key}" > /root/.ssh/authorized_keys
    chmod 600 /root/.ssh/authorized_keys
  EOT

  tags = {
    Name = "pluto"
  }
}

output "instance_id" {
  value = aws_instance.pluto.id
}

output "public_ip" {
  value = aws_instance.pluto.public_ip
}

output "deploy_command" {
  value = "deploy --hostname ${aws_instance.pluto.public_ip} .#pluto"
}
